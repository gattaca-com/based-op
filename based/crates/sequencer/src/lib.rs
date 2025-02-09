use std::{cmp, sync::Arc};

use alloy_primitives::B256;
use alloy_rpc_types::engine::{
    CancunPayloadFields, ExecutionPayload, ExecutionPayloadSidecar, ExecutionPayloadV3, ForkchoiceState,
};
use bop_common::{
    actor::Actor,
    communication::{
        messages::{
            self, BlockFetch, BlockSyncError, BlockSyncMessage, EngineApi, SimulatorToSequencer,
            SimulatorToSequencerMsg,
        },
        Connections, ReceiversSpine, SendersSpine, SpineConnections, TrackedSenders,
    },
    db::{DBFrag, DatabaseWrite},
    p2p::VersionedMessage,
    transaction::Transaction,
};
use bop_db::DatabaseRead;
use op_alloy_rpc_types_engine::{OpExecutionPayloadEnvelopeV3, OpPayloadAttributes};
use reth_optimism_primitives::OpTransactionSigned;
use reth_primitives::BlockWithSenders;
use reth_primitives_traits::SignedTransaction;
use revm::DatabaseRef;
use sorting::FragSequence;
use strum_macros::AsRefStr;
use tokio::sync::oneshot;

pub mod block_sync;
pub mod config;
mod context;
pub mod simulator;
pub(crate) mod sorting;

pub use config::SequencerConfig;
use context::SequencerContext;
pub use simulator::Simulator;
use sorting::SortingData;

pub fn payload_to_block(
    payload: ExecutionPayload,
    sidecar: ExecutionPayloadSidecar,
) -> Result<BlockSyncMessage, BlockSyncError> {
    let block = payload.try_into_block_with_sidecar::<OpTransactionSigned>(&sidecar)?;
    let block_senders = block
        .body
        .transactions
        .iter()
        .map(|tx| tx.recover_signer_unchecked())
        .collect::<Option<Vec<_>>>()
        .ok_or(BlockSyncError::SignerRecovery)?;
    Ok(BlockWithSenders { block, senders: block_senders })
}

pub struct Sequencer<Db> {
    state: SequencerState<Db>,
    data: SequencerContext<Db>,
}

impl<Db: DatabaseRead> Sequencer<Db> {
    pub fn new(db: Db, db_frag: DBFrag<Db>, config: SequencerConfig) -> Self {
        Self { state: SequencerState::default(), data: SequencerContext::new(db, db_frag, config) }
    }
}

impl<Db> Actor<Db> for Sequencer<Db>
where
    Db: DatabaseWrite + DatabaseRead,
{
    const CORE_AFFINITY: Option<usize> = Some(0);

    fn loop_body(&mut self, connections: &mut Connections<SendersSpine<Db>, ReceiversSpine<Db>>) {
        // handle new transaction
        while connections.receive(|msg, senders| {
            self.state.handle_new_tx(msg, &mut self.data, senders);
        }) {}

        // handle sim results
        while connections.receive(|msg, _| {
            let state = std::mem::take(&mut self.state);
            self.state = state.handle_sim_result(msg, &mut self.data);
        }) {}

        // handle engine API messages from rpc
        connections.receive(|msg: messages::EngineApi, senders| {
            let state = std::mem::take(&mut self.state);
            self.state = state.handle_engine_api(msg, &mut self.data, senders);
        });

        // handle block sync
        connections.receive(|msg, _| {
            let state = std::mem::take(&mut self.state);
            self.state = state.handle_block_sync(msg, &mut self.data);
        });

        // Check for passive state changes. e.g., sealing frags, sending sims, etc.
        let state = std::mem::take(&mut self.state);
        self.state = state.tick(&mut self.data, connections);
    }
}

/// Contains different states of the Sequencer state machine.
/// The state is stored as a reference in the Sequencer struct.
#[derive(Clone, Debug, Default, AsRefStr)]
pub enum SequencerState<Db> {
    /// Waiting for block sync
    Syncing {
        /// When the stage reaches this syncing is done
        /// TODO: not always true as we may have fallen behind the head - we should cache all payloads that come in
        /// while syncing
        last_block_number: u64,
    },
    /// Synced and waiting for the next new payload message
    #[default]
    WaitingForNewPayload,
    /// We've received a `NewPayload` message and are waiting for the fork choice to confirm the payload
    /// The two fields in this state are the payload and the sidecar.
    WaitingForForkChoice(ExecutionPayload, ExecutionPayloadSidecar),
    /// We've received a `NewPayload` and the FCU confirming that payload. We now get two potential messages after:
    /// - `ForkChoiceUpdatedV3 { payload_attributes: Some(attributes), .. }` This means we are the sequencer and we can
    ///   start building.
    /// - `NewPayloadV3 { .. }` This means we were not the sequencer for that block, and now need to sync this new
    ///   block and start the loop again.
    WaitingForForkChoiceWithAttributes,

    /// We've received a FCU with attributes and are now sequencing transactions into Frags.
    Sorting(FragSequence, SortingData<Db>),
}

impl<Db> SequencerState<Db>
where
    Db: DatabaseWrite + DatabaseRead,
{
    /// Processes Engine API messages that drive state transitions in the sequencer.
    ///
    /// State transitions follow this flow:
    /// 1. NewPayload -> WaitingForForkChoice
    /// 2. ForkChoice (no attributes) -> WaitingForForkChoiceWithAttributes
    /// 3. ForkChoice (with attributes) -> Sorting
    /// 4. GetPayload -> WaitingForNewPayload
    ///
    /// The Syncing state can be entered from any state if the chain falls behind.
    fn handle_engine_api(
        self,
        msg: EngineApi,
        data: &mut SequencerContext<Db>,
        senders: &SendersSpine<Db>,
    ) -> SequencerState<Db> {
        use EngineApi::*;

        tracing::info!("NewEngineApi: {:?}", msg.as_ref());

        match msg {
            NewPayloadV3 { payload, versioned_hashes, parent_beacon_block_root, .. } => {
                self.handle_new_payload_engine_api(payload, versioned_hashes, parent_beacon_block_root)
            }
            ForkChoiceUpdatedV3 { fork_choice_state, payload_attributes, .. } => {
                self.handle_fork_choice_updated_engine_api(fork_choice_state, payload_attributes, data, senders)
            }
            GetPayloadV3 { res, .. } => self.handle_get_payload_engine_api(res, data, senders),
        }
    }

    /// Processes new payload messages from the consensus layer.
    ///
    /// Validates the payload against current state and either:
    /// - Buffers it while waiting for fork choice confirmation
    /// - Triggers sync if we've fallen behind
    /// - Ignores if duplicate/old payload
    fn handle_new_payload_engine_api(
        self,
        payload: ExecutionPayloadV3,
        versioned_hashes: Vec<B256>,
        parent_beacon_block_root: B256,
    ) -> SequencerState<Db> {
        use SequencerState::*;

        match self {
            // Default path once synced. If `WaitingForNewPayload` we were the sequencer, if
            // `WaitingForForkChoiceWithAttributes` we were not the sequencer.
            WaitingForNewPayload | WaitingForForkChoiceWithAttributes => WaitingForForkChoice(
                ExecutionPayload::V3(payload),
                ExecutionPayloadSidecar::v3(CancunPayloadFields::new(parent_beacon_block_root, versioned_hashes)),
            ),
            // We are syncing and just got a new payload. Ignore it, we will bulk fetch this once we get a NewPayload
            // after this initial sync.
            Syncing { last_block_number } => {
                // TODO: buffer the payload
                Syncing { last_block_number }
            }
            WaitingForForkChoice(ref buffered_payload, _) => {
                match payload.payload_inner.payload_inner.block_number.cmp(&buffered_payload.block_number()) {
                    cmp::Ordering::Less => {
                        tracing::debug!("New Payload for an old block. Ignoring.");
                        self
                    }
                    cmp::Ordering::Equal => {
                        if buffered_payload.block_hash() == payload.payload_inner.payload_inner.block_hash {
                            tracing::debug!("Received 2 payloads with the same block hash. Ignoring latest.");
                            self
                        } else {
                            // We got 2 payloads for the same block. This shouldn't happen. TODO: handle this
                            // gracefully.
                            debug_assert!(false, "Received 2 payloads for the same block but different hashes.");
                            tracing::error!(
                                "Received 2 payloads for the same block but different hashes. Ignoring latest."
                            );
                            self
                        }
                    }
                    cmp::Ordering::Greater => {
                        // We are behind the head. Switch to syncing.
                        Syncing { last_block_number: buffered_payload.block_number() }
                    }
                }
            }

            Sorting(_, _) => {
                // This should never happen. We have been sequencing frags but haven't had GetPayload called before
                // NewPayload.
                debug_assert!(false, "Received NewPayload while in the wrong state");
                WaitingForForkChoice(
                    ExecutionPayload::V3(payload),
                    ExecutionPayloadSidecar::v3(CancunPayloadFields::new(parent_beacon_block_root, versioned_hashes)),
                )
            }
        }
    }

    /// Handles fork choice updates that confirm the canonical chain.
    ///
    /// Two types of updates:
    /// 1. Without attributes - Confirms previous payload and triggers state update
    /// 2. With attributes - Initiates new block building with provided parameters
    fn handle_fork_choice_updated_engine_api(
        self,
        fork_choice_state: ForkchoiceState,
        payload_attributes: Option<Box<OpPayloadAttributes>>,
        data: &mut SequencerContext<Db>,
        senders: &SendersSpine<Db>,
    ) -> SequencerState<Db> {
        use SequencerState::*;

        match self {
            WaitingForForkChoice(payload, sidecar) => {
                let head_bn = data.db.head_block_number().expect("couldn't get db");

                // FCU has skipped some blocks. Signal to fetch them all and set state to syncing.
                if payload.block_number() > head_bn + 1 {
                    let last_block_number = payload.block_number() - 1;
                    let _ = senders.send(BlockFetch::FromTo(head_bn + 1, last_block_number));
                    Syncing { last_block_number }
                } else {
                    // Confirm that the FCU payload is the same as the buffered payload.
                    if payload.block_hash() == fork_choice_state.head_block_hash {
                        let basefee = payload.as_v1().base_fee_per_gas;
                        let block = payload_to_block(payload, sidecar).expect("couldn't get block from payload");
                        data.commit_block(&block, Some(basefee.to()));
                        data.parent_header = block.header.clone();
                        data.parent_hash = fork_choice_state.head_block_hash;
                        WaitingForForkChoiceWithAttributes
                    } else {
                        // We have received the wrong ExecutionPayload. Need to re-sync with the new head.
                        // TODO: should fetch bn from the FCU head hash
                        tracing::warn!("Received wrong ExecutionPayload. Need to re-sync with the new head.");
                        let _ = senders.send(BlockFetch::FromTo(payload.block_number(), payload.block_number()));
                        Syncing { last_block_number: payload.block_number() }
                    }
                }
            }

            // Waiting for new payload should not happen, but while testing
            // we can basically keep sequencing based on the same db state
            WaitingForForkChoiceWithAttributes | WaitingForNewPayload => {
                match payload_attributes {
                    Some(attributes) => {
                        data.timers.start_sequencing.start();
                        let (seq, first_frag) = data.start_sequencing(attributes, senders);
                        data.timers.start_sequencing.stop();
                        tracing::info!("start sorting with {} orders", first_frag.tof_snapshot.len());
                        SequencerState::Sorting(seq, first_frag)
                    }
                    None => {
                        // We have got 2 FCU in a row with no attributes. This shouldn't happen?
                        debug_assert!(false, "Received 2 FCU in a row with no attributes");
                        tracing::warn!("Received 2 FCU in a row with no attributes");
                        self
                    }
                }
            }
            Syncing { .. } | Sorting(_, _) => {
                debug_assert!(false, "Received FCU in state {self:?}");
                tracing::warn!("Received FCU in state {self:?}");
                self
            }
        }
    }

    /// Finalises block production and returns the sealed payload.
    ///
    /// Critical path that:
    /// 1. Applies final transaction fragment
    /// 2. Seals the block
    /// 3. Broadcasts block data to p2p network
    /// 4. Returns payload to consensus layer
    fn handle_get_payload_engine_api(
        self,
        res: oneshot::Sender<OpExecutionPayloadEnvelopeV3>,
        data: &mut SequencerContext<Db>,
        senders: &SendersSpine<Db>,
    ) -> SequencerState<Db> {
        use SequencerState::*;

        match self {
            Sorting(seq, sorting_data) => {
                data.timers.waiting_for_sims.stop();
                data.timers.seal_block.start();
                let (frag, seal, block) = data.seal_block(seq, sorting_data);

                // Gossip seal to p2p and return payload to rpc
                let _ = senders.send(VersionedMessage::from(frag));
                let _ = senders.send(VersionedMessage::from(seal));
                let _ = res.send(block);
                data.timers.seal_block.stop();

                WaitingForNewPayload
            }
            s => {
                debug_assert!(false, "Should never have gotten here");
                s
            }
        }
    }

    /// Processes block sync messages during chain synchronisation.
    ///
    /// Applies blocks sequentially until reaching target height,
    /// then transitions back to normal operation.
    ///
    /// When we are syncing we fetch blocks asynchronously from the rpc and send them back through a channel that gets
    /// picked up and processed here.
    fn handle_block_sync(self, block: BlockSyncMessage, data: &mut SequencerContext<Db>) -> Self {
        use SequencerState::*;

        match self {
            Syncing { last_block_number } => {
                data.commit_block(&block, None);

                if block.number != last_block_number {
                    Syncing { last_block_number }
                } else {
                    // Wait until the next payload and attributes arrive
                    WaitingForNewPayload
                }
            }
            _ => {
                debug_assert!(false, "Should not have received block sync update while in state {self:?}");
                self
            }
        }
    }

    /// Sends a new transaction to the tx pool.
    /// If we are sorting, we pass Some(senders) to the tx pool so it can send top-of-frag simulations.
    fn handle_new_tx(&mut self, tx: Arc<Transaction>, data: &mut SequencerContext<Db>, senders: &SendersSpine<Db>) {
        data.tx_pool.handle_new_tx(
            tx.clone(),
            &data.db_frag,
            data.as_ref().basefee.to(),
            false,
            data.config.simulate_tof_in_pools.then_some(senders),
        );
        if let SequencerState::Sorting(_, sorting_data) = self {
            // This should ideally be not at the top but bottom of the sorted list. For now this is fastest
            sorting_data
                .tof_snapshot
                .push(bop_common::transaction::SimulatedTxList { current: None, pending: tx.into() });
        }
    }

    /// Processes transaction simulation results from the simulator actor.
    ///
    /// Handles both block transaction simulations during sorting and
    /// transaction pool simulations for future inclusion.
    fn handle_sim_result(mut self, result: SimulatorToSequencer, data: &mut SequencerContext<Db>) -> Self {
        let (sender, nonce) = result.sender_info;
        let state_id = result.state_id;
        let simtime = result.simtime;
        match result.msg {
            SimulatorToSequencerMsg::Tx(simulated_tx) => {
                let SequencerState::Sorting(_, sort_data) = &mut self else {
                    return self;
                };

                // handle sim on wrong state
                if !sort_data.is_valid(state_id) {
                    return self;
                }
                data.timers.handle_sim.start();
                sort_data.handle_sim(simulated_tx, &sender, data.as_ref().basefee.to(), simtime);
                data.timers.handle_sim.stop();
            }
            SimulatorToSequencerMsg::TxPoolTopOfFrag(simulated_tx) => {
                match simulated_tx {
                    Ok(res) if data.db_frag.is_valid(state_id) => data.tx_pool.handle_simulated(res),
                    Ok(_) => {
                        // No-op if the simulation is on a different fragment.
                        // We would have already re-sent the tx for sim on the correct fragment.
                    }
                    Err(_e) => {
                        // tracing::info!("simulation error for transaction pool tx {e}");
                        data.tx_pool.remove(&sender, nonce);
                    }
                }
            }
        }
        self
    }
}
impl<Db: Clone + DatabaseRef> SequencerState<Db> {
    /// Performs periodic state machine updates:
    ///
    /// - Seals transaction fragments when timing threshold reached
    /// - Triggers new transaction simulations when ready
    ///
    /// Used to maintain block production cadence.
    fn tick(self, data: &mut SequencerContext<Db>, connections: &mut SpineConnections<Db>) -> Self {
        use SequencerState::*;
        let base_fee = data.as_ref().basefee.to();
        match self {
            Sorting(mut seq, sorting_data) if sorting_data.should_seal_frag() => {
                data.timers.waiting_for_sims.stop();
                data.timers.seal_frag.start();
                data.tx_pool.remove_mined_txs(sorting_data.txs.iter());
                let (msg, new_sort_dat) = data.seal_frag(sorting_data, &mut seq);
                connections.send(VersionedMessage::from(msg));
                // Collect all transactions from the frag so we can use them to reset the tx pool.

                data.timers.seal_frag.stop();
                tracing::info!("start sorting with {} orders", new_sort_dat.tof_snapshot.len());
                Sorting(seq, new_sort_dat)
            }

            Sorting(seq, sorting_data) if sorting_data.should_send_next_sims() => {
                data.timers.apply_and_send_next.start();
                let new_sorting_data = sorting_data.apply_and_send_next(data.config.n_per_loop, connections, base_fee);
                if new_sorting_data.in_flight_sims > 1 {
                    data.timers.waiting_for_sims.stop();
                    data.timers.apply_and_send_next.stop();
                    data.timers.waiting_for_sims.start();
                }
                Sorting(seq, new_sorting_data)
            }

            _ => self,
        }
    }
}
