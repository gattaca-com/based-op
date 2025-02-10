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
    db::DatabaseWrite,
    p2p::{EnvV0, VersionedMessage},
    shared::SharedState,
    time::Duration,
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
    pub fn new(db: Db, shared_state: SharedState<Db>, config: SequencerConfig) -> Self {
        Self { state: SequencerState::default(), data: SequencerContext::new(db, shared_state, config) }
    }
}

impl<Db> Actor<Db> for Sequencer<Db>
where
    Db: DatabaseWrite + DatabaseRead,
{
    const CORE_AFFINITY: Option<usize> = Some(0);

    fn loop_body(&mut self, connections: &mut Connections<SendersSpine<Db>, ReceiversSpine<Db>>) {
        // handle block sync
        connections.receive_for(Duration::from_millis(10), |msg, senders| {
            let state = std::mem::take(&mut self.state);
            self.state = state.handle_block_sync(msg, &mut self.data, senders);
        });

        // handle new transaction
        connections.receive_for(Duration::from_millis(10), |msg, senders| {
            self.state.handle_new_tx(msg, &mut self.data, senders);
        });

        // handle sim results
        connections.receive_for(Duration::from_millis(10), |msg, _| {
            let state = std::mem::take(&mut self.state);
            self.state = state.handle_sim_result(msg, &mut self.data);
        });

        // handle engine API messages from rpc
        connections.receive_for(Duration::from_millis(10), |msg: messages::EngineApi, senders| {
            let state = std::mem::take(&mut self.state);
            self.state = state.handle_engine_api(msg, &mut self.data, senders);
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

    /// Synced and waiting for the next new payload message.
    #[default]
    WaitingForNewPayload,

    /// Wait for a FCU with attributes. This means we are the sequencer and we can start building.
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
        ctx: &mut SequencerContext<Db>,
        senders: &SendersSpine<Db>,
    ) -> SequencerState<Db> {
        use EngineApi::*;

        tracing::info!("NewEngineApi: {:?}", msg.as_ref());

        match msg {
            NewPayloadV3 { payload, versioned_hashes, parent_beacon_block_root, .. } => {
                self.handle_new_payload_engine_api(ctx, senders, payload, versioned_hashes, parent_beacon_block_root)
            }
            ForkChoiceUpdatedV3 { fork_choice_state, payload_attributes, .. } => {
                self.handle_fork_choice_updated_engine_api(fork_choice_state, payload_attributes, ctx, senders)
            }
            GetPayloadV3 { res, .. } => self.handle_get_payload_engine_api(res, ctx, senders),
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
        ctx: &mut SequencerContext<Db>,
        senders: &SendersSpine<Db>,
        payload: ExecutionPayloadV3,
        versioned_hashes: Vec<B256>,
        parent_beacon_block_root: B256,
    ) -> SequencerState<Db> {
        use SequencerState::*;

        match self {
            // We are syncing and just got a new payload. Ignore it, we will bulk fetch this once we get a NewPayload
            // after this initial sync.
            Syncing { last_block_number } => {
                // TODO: buffer the payload
                Syncing { last_block_number }
            }

            // Default path once synced. Apply and commit the payload.
            WaitingForNewPayload | WaitingForForkChoiceWithAttributes => {
                let head_bn = ctx.db.head_block_number().expect("couldn't get db");
                let payload = ExecutionPayload::V3(payload);
                let sidecar =
                    ExecutionPayloadSidecar::v3(CancunPayloadFields::new(parent_beacon_block_root, versioned_hashes));

                // NewPayload skipped some blocks. Signal to fetch them all and set state to syncing.
                if payload.block_number() > head_bn + 1 {
                    let last_block_number = payload.block_number() - 1;
                    let _ = senders.send(BlockFetch::FromTo(head_bn + 1, last_block_number));
                    Syncing { last_block_number }
                } else {
                    let payload_hash = payload.block_hash();
                    // Check if we have already committed this payload.
                    if payload_hash == ctx.db.head_block_hash().expect("couldn't get db head block hash") {
                        return WaitingForForkChoiceWithAttributes;
                    }

                    let block = payload_to_block(payload, sidecar).expect("couldn't get block from payload");

                    // Update sorting context
                    ctx.parent_header = block.header.clone();
                    ctx.parent_hash = payload_hash;

                    // Commit the block
                    if let Some(block_fetch) = ctx.commit_block(&block) {
                        let fetching_to = block_fetch.fetch_to();
                        let _ = senders.send(block_fetch);
                        Syncing { last_block_number: fetching_to }
                    } else {
                        WaitingForForkChoiceWithAttributes
                    }
                }
            }
            Sorting(_, _) => {
                tracing::warn!("Received NewPayload when state is Sorting {self:?}");
                self
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
        ctx: &mut SequencerContext<Db>,
        senders: &SendersSpine<Db>,
    ) -> SequencerState<Db> {
        use SequencerState::*;

        match self {
            // Waiting for new payload should not happen, but while testing
            // we can basically keep sequencing based on the same db state
            WaitingForForkChoiceWithAttributes | WaitingForNewPayload => {
                if matches!(self, WaitingForNewPayload) {
                    //TODO: This should never happen in production but for benchmarking it allows us to keep simming on
                    // top of the same block!
                    ctx.tx_pool.clear();
                    ctx.deposits.clear();
                    ctx.shared_state.as_mut().reset();
                }

                match payload_attributes {
                    Some(attributes) => {
                        ctx.timers.start_sequencing.start();
                        let (seq, first_frag) = ctx.start_sequencing(attributes, senders);
                        ctx.timers.start_sequencing.stop();

                        let env_msg: EnvV0 = (&ctx.block_env).into();
                        let _ = senders.send(VersionedMessage::from(env_msg));

                        tracing::info!("start sorting with {} orders", first_frag.tof_snapshot.len());
                        SequencerState::Sorting(seq, first_frag)
                    }
                    None => {
                        // Check that we are at this head.
                        let head_block_hash = ctx.db.head_block_hash().expect("couldn't get db head block hash");
                        if fork_choice_state.head_block_hash != head_block_hash {
                            // We are on the wrong head. Switch to syncing and request the head block.
                            let head_block_number =
                                ctx.db.head_block_number().expect("couldn't get db head block number");
                            let _ = senders.send(BlockFetch::FromTo(head_block_number, head_block_number));
                            Syncing { last_block_number: head_block_number }
                        } else {
                            WaitingForForkChoiceWithAttributes
                        }
                    }
                }
            }

            Syncing { last_block_number } => Syncing { last_block_number },

            Sorting(_, _) => {
                tracing::warn!("Received FCU when sorting {self:?}. Syncing to new head.");
                Syncing {
                    last_block_number: ctx.db.head_block_number().expect("couldn't get db head block number") + 1,
                }
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
        ctx: &mut SequencerContext<Db>,
        senders: &SendersSpine<Db>,
    ) -> SequencerState<Db> {
        use SequencerState::*;

        match self {
            Sorting(mut seq, sorting_data) => {
                ctx.timers.waiting_for_sims.stop();
                ctx.timers.seal_block.start();

                // Gossip last frag before sealing
                let last_frag = ctx.seal_last_frag(&mut seq, sorting_data);
                let s = senders.send_timeout(VersionedMessage::from(last_frag), Duration::from_millis(10));
                debug_assert!(s.is_ok(), "couldn't send last frag for 10 millis");

                let (seal, block) = ctx.seal_block(seq);

                // Gossip seal to p2p and return payload to rpc
                let s = senders.send_timeout(VersionedMessage::from(seal), Duration::from_millis(10));
                debug_assert!(s.is_ok(), "couldn't send seal for 10 millis");
                let s = res.send(block.clone());
                debug_assert!(s.is_ok(), "couldn't send block envelope to rpc");
                ctx.timers.seal_block.stop();

                // Commit the block to the db
                if ctx.config.commit_sealed_frags_to_db {
                    let sidecar =
                        ExecutionPayloadSidecar::v3(CancunPayloadFields::new(block.parent_beacon_block_root, vec![]));
                    let block = payload_to_block(ExecutionPayload::V3(block.execution_payload), sidecar)
                        .expect("couldn't get block from payload");
                    ctx.commit_block(&block);
                }

                WaitingForNewPayload
            }
            s => s,
        }
    }

    /// Processes block sync messages during chain synchronisation.
    ///
    /// Applies blocks sequentially until reaching target height,
    /// then transitions back to normal operation.
    ///
    /// When we are syncing we fetch blocks asynchronously from the rpc and send them back through a channel that gets
    /// picked up and processed here.
    fn handle_block_sync(
        self,
        block: BlockSyncMessage,
        ctx: &mut SequencerContext<Db>,
        senders: &SendersSpine<Db>,
    ) -> Self {
        use SequencerState::*;

        match self {
            Syncing { last_block_number } => {
                if let Some(blocks_to_fetch) = ctx.commit_block(&block) {
                    let fetching_to = blocks_to_fetch.fetch_to();
                    let _ = senders.send(blocks_to_fetch);
                    Syncing { last_block_number: fetching_to.max(last_block_number) }
                } else if block.number != last_block_number {
                    Syncing { last_block_number }
                } else {
                    // Wait until the next payload and attributes arrive
                    WaitingForNewPayload
                }
            }

            WaitingForNewPayload | WaitingForForkChoiceWithAttributes => {
                ctx.commit_block(&block);
                WaitingForNewPayload
            }
            _ => {
                debug_assert!(false, "Should not have received block sync update while in state {self:?}");
                self
            }
        }
    }

    /// Sends a new transaction to the tx pool.
    /// If we are sorting, we pass Some(senders) to the tx pool so it can send top-of-frag simulations.
    fn handle_new_tx(&mut self, tx: Arc<Transaction>, ctx: &mut SequencerContext<Db>, senders: &SendersSpine<Db>) {
        if tx.is_deposit() {
            ctx.deposits.push_back(tx);
            return;
        }
        ctx.tx_pool.handle_new_tx(
            tx.clone(),
            ctx.shared_state.as_ref(),
            ctx.as_ref().basefee.to(),
            false,
            ctx.config.simulate_tof_in_pools.then_some(senders),
        );
        if let SequencerState::Sorting(_, sorting_data) = self {
            // This should ideally be not at the top but bottom of the sorted list. For now this is fastest
            sorting_data
                .tof_snapshot
                .push_front(bop_common::transaction::SimulatedTxList { current: None, pending: tx.into() });
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
                    Ok(res) if data.shared_state.as_ref().is_valid(state_id) => data.tx_pool.handle_simulated(res),
                    Ok(_) => {
                        // No-op if the simulation is on a different fragment.
                        // We would have already re-sent the tx for sim on the correct fragment.
                    }
                    Err(_e) => {
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
                // Reset the tx pool.
                data.tx_pool.remove_mined_txs(sorting_data.txs.iter());
                let (msg, new_sort_dat) = data.seal_frag(sorting_data, &mut seq);
                connections.send(VersionedMessage::from(msg));

                data.timers.seal_frag.stop();
                tracing::info!("start sorting with {} orders", new_sort_dat.tof_snapshot.len());
                Sorting(seq, new_sort_dat)
            }

            Sorting(seq, mut sorting_data) if sorting_data.should_send_next_sims() => {
                sorting_data.maybe_apply(base_fee);

                data.timers.handle_deposits.start();
                sorting_data.handle_deposits(&mut data.deposits, connections);
                data.timers.handle_deposits.stop();

                data.timers.send_next.start();
                let new_sorting_data = sorting_data.send_next(data.config.n_per_loop, connections);
                if new_sorting_data.in_flight_sims > 1 {
                    data.timers.waiting_for_sims.stop();
                    data.timers.send_next.stop();
                    data.timers.waiting_for_sims.start();
                }
                Sorting(seq, new_sorting_data)
            }

            _ => self,
        }
    }
}
