use std::{cmp, sync::Arc};

use alloy_primitives::B256;
use alloy_rpc_types::engine::{
    CancunPayloadFields, ExecutionPayload, ExecutionPayloadSidecar, ExecutionPayloadV3, ForkchoiceState,
};
use block_sync::BlockSync;
use bop_common::{
    actor::Actor,
    communication::{
        messages::{
            self, BlockFetch, BlockSyncError, BlockSyncMessage, EngineApi, SimulatorToSequencer, SimulatorToSequencerMsg
        },
        Connections, ReceiversSpine, SendersSpine, SpineConnections, TrackedSenders,
    },
    db::{DBFrag, DatabaseWrite},
    p2p::VersionedMessage,
    time::Duration,
    transaction::Transaction,
};
use bop_db::DatabaseRead;
use op_alloy_rpc_types_engine::{OpExecutionPayloadEnvelopeV3, OpPayloadAttributes};
use reth_evm::{ConfigureEvmEnv, NextBlockEnvAttributes};
use reth_optimism_primitives::OpTransactionSigned;
use reth_primitives::BlockWithSenders;
use reth_primitives_traits::SignedTransaction;
use sorting::FragSequence;
use strum_macros::AsRefStr;
use tokio::sync::oneshot;
use tracing::warn;

pub mod block_sync;
pub mod config;
mod context;
pub(crate) mod sorting;

pub use config::SequencerConfig;
use context::SequencerContext;
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

#[derive(Clone, Debug)]
pub struct Sequencer<Db: DatabaseWrite + DatabaseRead> {
    state: SequencerState<Db>,
    data: SequencerContext<Db>,
}

impl<Db: DatabaseWrite + DatabaseRead> Sequencer<Db> {
    pub fn new(db: Db, frag_db: DBFrag<Db>, config: SequencerConfig) -> Self {
        let frags = FragSequence::new(frag_db, 0); // TODO: move to shared state
        let block_executor = BlockSync::new(config.evm_config.chain_spec().clone());

        Self {
            state: SequencerState::default(),

            data: SequencerContext {
                db,
                frags,
                block_executor,
                config,
                tx_pool: Default::default(),
                fork_choice_state: Default::default(),
                payload_attributes: Default::default(),
                base_fee: Default::default(),
                parent_hash: Default::default(),
                parent_header: Default::default(),
                block_env: Default::default(),
            },
        }
    }
}

impl<Db> Actor<Db> for Sequencer<Db>
where
    Db: DatabaseWrite + DatabaseRead,
{
    const CORE_AFFINITY: Option<usize> = Some(0);

    fn loop_body(&mut self, connections: &mut Connections<SendersSpine<Db>, ReceiversSpine<Db>>) {
        // handle new transaction
        connections.receive(|msg, senders| {
            self.state.handle_new_tx(msg, &mut self.data, senders);
        });

        // handle sim results
        connections.receive(|msg, _| {
            self.state.handle_sim_result(msg, &mut self.data);
        });

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
pub enum SequencerState<Db: DatabaseWrite + DatabaseRead> {
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
    Sorting(SortingData<Db>),
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
            Sorting(_) => {
                // This should never happen. We have been sequencing frags but haven't had GetPayload called before
                // NewPayload.
                debug_assert!(false, "Received NewPayload while sorting");
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
                        let block = payload_to_block(payload, sidecar).expect("couldn't get block from payload");
                        let _ = data.block_executor.apply_and_commit_block(&block, &data.db, true);
                        data.reset_fragdb();
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
            WaitingForForkChoiceWithAttributes => {
                match payload_attributes {
                    Some(attributes) => {
                        // From: https://specs.optimism.io/protocol/exec-engine.html#extended-payloadattributesv2
                        // The gasLimit is optional w.r.t. compatibility with L1, but required when used as rollup. This
                        // field overrides the gas limit used during block-building. If not
                        // specified as rollup, a STATUS_INVALID is returned.
                        let gas_limit = attributes.gas_limit.unwrap();
                        let next_attr = NextBlockEnvAttributes {
                            timestamp: attributes.payload_attributes.timestamp,
                            suggested_fee_recipient: attributes.payload_attributes.suggested_fee_recipient,
                            prev_randao: attributes.payload_attributes.prev_randao,
                            gas_limit,
                        };

                        let block_env = data
                            .config
                            .evm_config
                            .next_cfg_and_block_env(&data.parent_header, next_attr)
                            .expect("couldn't create blockenv");
                        // should never fail as its a broadcast
                        senders
                            .send_timeout(block_env.block_env.clone(), Duration::from_millis(10))
                            .expect("couldn't send block env");
                        data.block_env = block_env.block_env;

                        let txs = attributes
                            .transactions
                            .map(|txs| {
                                txs.into_iter().map(|bytes| Transaction::decode(bytes).unwrap().into()).collect()
                            })
                            .unwrap_or_default();

                        // TODO: remove gas limit from frags
                        data.frags.set_gas_limit(gas_limit);

                        let sorting_data = data.new_sorting_data(txs, !attributes.no_tx_pool.unwrap_or(false));
                        Sorting(sorting_data)
                    }
                    None => {
                        // We have got 2 FCU in a row with no attributes. This shouldn't happen?
                        debug_assert!(false, "Received 2 FCU in a row with no attributes");
                        tracing::warn!("Received 2 FCU in a row with no attributes");
                        self
                    }
                }
            }
            Syncing { .. } | Sorting(_) |  WaitingForNewPayload => {
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
            Sorting(sorting_data) => {
                // Apply final frag to db and send frag to p2p
                let mut frag = data.frags.apply_sorted_frag(sorting_data.frag);
                frag.is_last = true;
                let frag_msg = VersionedMessage::from(frag);
                let _ = senders.send(frag_msg);

                let (seal, block) =
                    data.frags.seal_block(&data.block_env, data.config.evm_config.chain_spec(), data.parent_hash);

                // Gossip seal to p2p and return payload to rpc
                let _ = senders.send(VersionedMessage::from(seal));
                let _ = res.send(block);

                WaitingForNewPayload
            }
            _ => {
                tracing::warn!("Received GetPayload in state {self:?}");
                // TODO: impl error in res
                WaitingForNewPayload
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
                data.block_executor.apply_and_commit_block(&block, &data.db, true).expect("issue syncing block");
                data.reset_fragdb();

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
    fn handle_new_tx(&mut self, msg: Arc<Transaction>, data: &mut SequencerContext<Db>, senders: &SendersSpine<Db>) {
        let senders = matches!(self, SequencerState::Sorting(_)).then_some(senders);
        data.tx_pool.handle_new_tx(msg, data.frags.db_ref(), data.base_fee, senders);
    }

    /// Processes transaction simulation results from the simulator actor.
    ///
    /// Handles both block transaction simulations during sorting and
    /// transaction pool simulations for future inclusion.
    fn handle_sim_result(&mut self, result: SimulatorToSequencer<Db>, data: &mut SequencerContext<Db>) {
        let (sender, nonce) = result.sender_info;
        match result.msg {
            SimulatorToSequencerMsg::Tx(simulated_tx) => {
                let SequencerState::Sorting(sort_data) = self else {
                    return;
                };

                // handle sim on wrong state
                if sort_data.is_valid(result.state_id) {
                    warn!("received sim result on wrong state, dropping");
                    return;
                }
                sort_data.handle_sim(simulated_tx, &sender, data.base_fee);
            }
            SimulatorToSequencerMsg::TxPoolTopOfFrag(simulated_tx) => {
                match simulated_tx {
                    Ok(res) if data.frags.is_valid(result.state_id) => data.tx_pool.handle_simulated(res),
                    Ok(_) => {
                        // No-op if the simulation is on a different fragment.
                        // We would have already re-sent the tx for sim on the correct fragment.
                    }
                    Err(e) => {
                        tracing::debug!("simulation error for transaction pool tx {e}");
                        data.tx_pool.remove(&sender, nonce);
                    }
                }
            }
        }
    }

    /// Performs periodic state machine updates:
    /// - Seals transaction fragments when timing threshold reached
    /// - Triggers new transaction simulations when ready
    ///
    /// Used to maintain block production cadence.
    fn tick(self, data: &mut SequencerContext<Db>, connections: &mut SpineConnections<Db>) -> Self {
        use SequencerState::*;
        match self {
            Sorting(sorting_data) if sorting_data.should_seal_frag() => {
                let frag = data.frags.apply_sorted_frag(sorting_data.frag);
                // broadcast to p2p
                connections.send(VersionedMessage::from(frag));
                let sorting_data =
                    data.new_sorting_data(sorting_data.remaining_attributes_txs, sorting_data.can_add_txs);
                Sorting(sorting_data.apply_and_send_next(data.config.n_per_loop, connections, data.base_fee))
            }

            Sorting(sorting_data) if sorting_data.should_send_next_sims() => {
                Sorting(sorting_data.apply_and_send_next(data.config.n_per_loop, connections, data.base_fee))
            }

            _ => self,
        }
    }
}
