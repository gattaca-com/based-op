use std::sync::Arc;

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
    time::{Duration, Repeater},
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
use tracing::{info, warn};

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
    heartbeat: Repeater,
}

impl<Db: DatabaseRead> Sequencer<Db> {
    pub fn new(db: Db, shared_state: SharedState<Db>, config: SequencerConfig) -> Self {
        Self {
            state: SequencerState::default(),
            data: SequencerContext::new(db, shared_state, config),
            heartbeat: Repeater::every(Duration::from_secs(2)),
        }
    }
}

impl<Db> Actor<Db> for Sequencer<Db>
where
    Db: DatabaseWrite + DatabaseRead,
{
    fn loop_body(&mut self, connections: &mut Connections<SendersSpine<Db>, ReceiversSpine<Db>>) {
        let block_sync_receive_duration = if matches!(self.state, SequencerState::Syncing { .. }) {
            // we're syncing anyway
            Duration::MAX
        } else {
            Duration::from_millis(10)
        };
        // handle block sync
        connections.receive_for(block_sync_receive_duration, |msg, senders| {
            let state = std::mem::take(&mut self.state);
            self.state = state.handle_block_sync(msg, &mut self.data, senders);
        });

        // handle new transaction
        connections.receive(|msg, senders| {
            self.state.handle_new_tx(msg, &mut self.data, senders);
        });

        // handle sim results
        connections.receive_for(Duration::from_millis(10), |msg, _| {
            let state = std::mem::take(&mut self.state);
            self.state = state.handle_sim_result(msg, &mut self.data);
        });

        // handle engine API messages from rpc
        connections.receive(|msg: messages::EngineApi, senders| {
            let state = std::mem::take(&mut self.state);
            self.state = state.handle_engine_api(msg, &mut self.data, senders);
        });

        // Check for passive state changes. e.g., sealing frags, sending sims, etc.
        let state = std::mem::take(&mut self.state);
        self.state = state.tick(&mut self.data, connections);

        if self.heartbeat.fired() {
            info!("in state {}", self.state.as_ref())
        }
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

impl<Db> SequencerState<Db> {
    fn sync_until(start: u64, stop: u64, senders: &SendersSpine<Db>) -> Self {
        let s = senders.send_timeout(BlockFetch::FromTo(start, stop), Duration::from_millis(10));
        debug_assert!(s.is_ok(), "Coulnd't send BlockFetch for more than 10 millis");
        info!("Start Syncing from {start} to {stop}");
        Self::Syncing { last_block_number: stop }
    }
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
        if matches!(self, Sorting(_, _)) {
            warn!("Received NewPayload when state is Sorting. This is normally not a problem, but rare nonetheless.");
            return self;
        }
        let head_bn = ctx.db.head_block_number().expect("couldn't get db");
        let bn = payload.payload_inner.payload_inner.block_number;
        if bn > head_bn + 1 {
            return Self::sync_until(head_bn + 1, bn, senders);
        };

        match self {
            // Default path once synced. Apply and commit the payload.
            WaitingForNewPayload | WaitingForForkChoiceWithAttributes => {
                let payload = ExecutionPayload::V3(payload);
                let sidecar =
                    ExecutionPayloadSidecar::v3(CancunPayloadFields::new(parent_beacon_block_root, versioned_hashes));

                // Clear shared state for each NewPayload event
                ctx.shared_state.reset();

                // NewPayload skipped some blocks. Signal to fetch them all and set state to syncing.
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
                if let Some((start, stop)) = ctx.commit_block(&block) {
                    Self::sync_until(start, stop, senders)
                } else {
                    WaitingForForkChoiceWithAttributes
                }
            }
            _ => self,
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
                        // Don't start sequencing until we have a parent hash.
                        if ctx.parent_header.parent_hash == B256::ZERO {
                            return self;
                        }

                        ctx.timers.start_sequencing.start();
                        let (seq, first_frag) = ctx.start_sequencing(attributes, senders);
                        ctx.timers.start_sequencing.stop();

                        let env_msg: EnvV0 = EnvV0::new(
                            &ctx.block_env,
                            ctx.parent_hash,
                            &ctx.extra_data(),
                            ctx.parent_beacon_block_root().unwrap(),
                        );
                        let _ = senders.send(VersionedMessage::from(env_msg));

                        info!("start sorting with {} orders", first_frag.tof_snapshot.len());
                        SequencerState::Sorting(seq, first_frag)
                    }
                    None => {
                        // Check that we are at this head.
                        let head_block_hash = ctx.db.head_block_hash().expect("couldn't get db head block hash");
                        if fork_choice_state.head_block_hash != head_block_hash {
                            // We are on the wrong head. Switch to syncing and request the head block.
                            let head_block_number =
                                ctx.db.head_block_number().expect("couldn't get db head block number");
                            ctx.shared_state.reset();
                            Self::sync_until(head_block_number, head_block_number, senders)
                        } else {
                            WaitingForForkChoiceWithAttributes
                        }
                    }
                }
            }

            Sorting(frag_seq, data) => {
                let head_block_hash = ctx.db.head_block_hash().expect("couldn't get db head block hash");
                if fork_choice_state.head_block_hash == head_block_hash {
                    return Sorting(frag_seq, data);
                }
                warn!("received FCU when Sorting. Sending already Fragged txs back to the pools and syncing to the new head.");
                for tx in frag_seq.txs.into_iter() {
                    ctx.handle_tx(tx.tx, senders);
                }
                let start = ctx.db.head_block_number().expect("couldn't get db head block number");
                let stop = start + 1;
                Self::sync_until(start, stop, senders)
            }
            _ => self,
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
                    ctx.shared_state.reset();
                    info!("committing to db");
                    return WaitingForForkChoiceWithAttributes;
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
                if let Some((start, stop)) = ctx.commit_block(&block) {
                    Self::sync_until(start, last_block_number.max(stop), senders)
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
        if let SequencerState::Sorting(_, sorting_data) = self {
            sorting_data
                .tof_snapshot
                .push_front(bop_common::transaction::SimulatedTxList { current: None, pending: tx.clone().into() });
        }
        ctx.handle_tx(tx, senders);
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
                sort_data.handle_sim(simulated_tx, sender, data.as_ref().basefee.to(), simtime);
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
                info!("start sorting with {} orders", new_sort_dat.tof_snapshot.len());
                Sorting(seq, new_sort_dat)
            }

            Sorting(seq, mut sorting_data) if sorting_data.should_send_next_sims() => {
                sorting_data.maybe_apply(base_fee);

                data.timers.handle_deposits.start();
                sorting_data.handle_deposits(&mut data.deposits, connections);
                data.timers.handle_deposits.stop();

                data.timers.send_next.start();
                sorting_data.send_next(data.config.n_per_loop, connections);
                if sorting_data.in_flight_sims > 1 {
                    data.timers.waiting_for_sims.stop();
                    data.timers.send_next.stop();
                    data.timers.waiting_for_sims.start();
                }
                Sorting(seq, sorting_data)
            }

            _ => self,
        }
    }
}
