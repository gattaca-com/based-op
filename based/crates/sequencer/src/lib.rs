use std::sync::Arc;

use alloy_consensus::Block;
use alloy_rpc_types::engine::ForkchoiceState;
use bop_common::{
    actor::Actor,
    communication::{
        messages::{self, SequencerToSimulator, SimulationError, SimulatorToSequencer},
        Connections, ReceiversSpine, SendersSpine, TrackedSenders,
    },
    db::{BopDB, BopDbRead, DBFrag, DBSorting},
    time::{Duration, Instant},
    transaction::{SimulatedTx, SimulatedTxList, Transaction},
};
use bop_pool::transaction::pool::TxPool;
use built_frag::BuiltFrag;
use op_alloy_rpc_types_engine::OpPayloadAttributes;
use reth_optimism_node::OpPayloadBuilderAttributes;
use reth_optimism_primitives::OpTransactionSigned;
use reth_primitives::BlockWithSenders;
use revm::db::CacheDB;
use revm_primitives::{db::DatabaseRef, Address, B256};
use strum_macros::AsRefStr;
use tokio::runtime::Runtime;
use tracing::{error, info, warn, Instrument};

use crate::block_sync::fetch_blocks::fetch_blocks_and_send_sequentially;

pub(crate) mod block_sync;
pub(crate) mod built_frag;

#[derive(Clone, Debug, Default)]
pub struct SortingFragOrders {
    orders: Vec<SimulatedTxList>,
}

impl SortingFragOrders {
    fn len(&self) -> usize {
        self.orders.len()
    }

    pub fn remove_from_sender(&mut self, sender: &Address) {
        for i in (0..self.len() - 1).rev() {
            let order = &mut self.orders[i];
            if &order.sender() == sender {
                if order.pop() {
                    self.orders.swap_remove(i);
                    return;
                }
            }
        }
    }

    pub fn remove_hash(&mut self, hash: &B256) {
        for i in (0..self.len() - 1).rev() {
            let order = &mut self.orders[i];
            if &order.hash() == hash {
                if order.pop() {
                    self.orders.swap_remove(i);
                    return;
                }
            }
        }
    }

    pub fn put(&mut self, tx: SimulatedTx) {
        let sender = tx.sender();
        for order in self.orders.iter_mut().rev() {
            if order.sender() == sender {
                order.current = Some(tx);
                return;
            }
        }
    }
}

impl<Db: BopDB> From<&SharedData<Db>> for SortingFragOrders {
    fn from(value: &SharedData<Db>) -> Self {
        Self { orders: value.tx_pool.clone_active() }
    }
}

#[derive(Clone, Debug)]
struct SortingData<Db: BopDB> {
    /// This is the db that is built on top of the last block chunk to be used to
    /// build a new cachedb on top of for sorting
    /// starting a new sort
    frag: BuiltFrag<Db::ReadOnly>,
    until: Instant,
    in_flight_sims: usize,
    tof_snapshot: SortingFragOrders,
    next_to_be_applied: Option<SimulatedTx>,
}
impl<Db: BopDB> SortingData<Db> {
    pub fn apply_and_send_next(&mut self, n_sims_per_loop: usize, senders: &SendersSpine<Db::ReadOnly>) {
        if let Some(tx_to_apply) = std::mem::take(&mut self.next_to_be_applied) {
            self.tof_snapshot.remove_from_sender(&tx_to_apply.sender());
            self.frag.apply_tx(tx_to_apply);
        }
    }
}

impl<Db: BopDB> From<&SharedData<Db>> for SortingData<Db> {
    fn from(data: &SharedData<Db>) -> Self {
        Self {
            frag: BuiltFrag::new(data.frag_db.clone().into(), data.config.max_gas),
            until: Instant::now() + data.config.frag_duration,
            in_flight_sims: 0,
            next_to_be_applied: None,
            tof_snapshot: data.into(),
        }
    }
}

#[derive(Clone, Debug, Default, AsRefStr)]
pub enum SequencerState<Db: BopDB> {
    #[default]
    WaitingForSync,
    WaitingForPayloadAttributes,
    Sorting(SortingData<Db>),
    Syncing {
        /// When the stage reaches this syncing is done
        last_block_number: u64,
    },
}

#[derive(Debug, AsRefStr)]
#[repr(u8)]
pub enum SequencerEvent<Db: BopDbRead> {
    BlockSync(Result<BlockWithSenders<Block<OpTransactionSigned>>, reqwest::Error>),
    ReceivedPayloadAttribues(Option<Box<OpPayloadBuilderAttributes>>),
    NewTx(Arc<Transaction>),
    SimResult(SimulatorToSequencer<Db>),
}

impl<Db: BopDB> SequencerState<Db> {
    pub fn start_sorting(data: &SharedData<Db>) -> Self {
        Self::Sorting(data.into())
    }

    fn handle_block_sync(
        mut self,
        block: Result<BlockWithSenders<Block<OpTransactionSigned>>, reqwest::Error>,
        data: &mut SharedData<Db>,
    ) -> Self {
        todo!()
    }

    fn handle_payload_attributes(
        mut self,
        attributes: Option<Box<OpPayloadBuilderAttributes>>,
        data: &mut SharedData<Db>,
    ) -> Self {
        todo!()
    }

    fn handle_new_tx(
        mut self,
        tx: Arc<Transaction>,
        data: &mut SharedData<Db>,
        senders: &SendersSpine<Db::ReadOnly>,
    ) -> Self {
        data.tx_pool.handle_new_tx(tx, &data.frag_db, DEFAULT_BASE_FEE, senders);
        self
    }

    fn handle_sim_result(
        self,
        data: &mut SharedData<Db>,
        result: SimulatorToSequencer<Db::ReadOnly>,
        senders: &SendersSpine<Db::ReadOnly>,
    ) -> Self {
        use messages::SimulatorToSequencerMsg::*;
        let sender = *result.sender();
        match result.msg {
            Tx(simulated_tx) => {
                // make sure we are actually sorting
                let SequencerState::Sorting(mut sort_data) = self else {
                    return self;
                };

                // handle sim on wrong state
                if result.unique_hash != sort_data.frag.db.unique_hash() {
                    warn!("received sim result on wrong state, dropping");
                    return SequencerState::Sorting(sort_data);
                }

                sort_data.in_flight_sims -= 1;

                // handle errored sim
                let Ok(simulated_tx) = simulated_tx.inspect_err(|e| error!("simming tx for sender {sender} {e}",))
                else {
                    sort_data.tof_snapshot.remove_from_sender(&sender);
                    return SequencerState::Sorting(sort_data);
                };

                let tx_to_put_back =
                    if sort_data.next_to_be_applied.as_ref().is_none_or(|t| t.payment < simulated_tx.payment) {
                        sort_data.next_to_be_applied.replace(simulated_tx)
                    } else {
                        Some(simulated_tx)
                    };
                if let Some(tx) = tx_to_put_back {
                    sort_data.tof_snapshot.put(tx)
                }
                SequencerState::Sorting(sort_data)
            }
            TxTof(simulated_tx) => {
                match simulated_tx {
                    Ok(res) if result.unique_hash == data.frag_db.unique_hash() => data.tx_pool.handle_simulated(res),
                    // resend because was on the wrong hash
                    Ok(res) => {
                        let _ = senders.send_timeout(
                            SequencerToSimulator::SimulateTxTof(res.tx, data.frag_db.clone()),
                            Duration::from_millis(10),
                        );
                    }
                    Err(e) => {
                        error!("simming tx {e}");
                        data.tx_pool.remove(&sender)
                    }
                }
                self
            }
        }
    }

    fn _update(mut self, data: &mut SharedData<Db>, senders: &SendersSpine<Db::ReadOnly>) -> Self {
        use SequencerState::*;
        match self {
            Sorting(sorting_data) if sorting_data.until < Instant::now() => {
                todo!("Seal and send frag")
            }
            Sorting(mut sorting_data) if sorting_data.in_flight_sims == 0 => {
                sorting_data.apply_and_send_next(data.config.n_per_loop, senders);
                Sorting(sorting_data)
            }
            _ => self,
        }
    }

    pub fn update(
        mut self,
        event: SequencerEvent<Db::ReadOnly>,
        data: &mut SharedData<Db>,
        senders: &SendersSpine<Db::ReadOnly>,
    ) -> Self {
        use SequencerEvent::*;
        match event {
            BlockSync(block) => self.handle_block_sync(block, data),
            ReceivedPayloadAttribues(attributes) => self.handle_payload_attributes(attributes, data),
            NewTx(tx) => self.handle_new_tx(tx, data, senders),
            SimResult(res) => self.handle_sim_result(data, res, senders),
        }
        ._update(data, senders)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SequencerConfig {
    frag_duration: Duration,
    max_gas: u64,
    n_per_loop: usize,
}
impl Default for SequencerConfig {
    fn default() -> Self {
        Self { frag_duration: Duration::from_millis(200), max_gas: 300_000_000, n_per_loop: 10 }
    }
}

#[derive(Clone, Debug)]
pub struct SharedData<Db: BopDB> {
    tx_pool: TxPool,
    db: Db,
    frag_db: DBFrag<Db::ReadOnly>,
    runtime: Arc<Runtime>,
    config: SequencerConfig,
    fork_choice_state: ForkchoiceState,
    payload_attributes: Box<OpPayloadAttributes>,
}

#[derive(Clone, Debug)]
pub struct Sequencer<Db: BopDB> {
    state: SequencerState<Db>,
    data: SharedData<Db>,
}

impl<Db: BopDB> Sequencer<Db> {
    pub fn new(db: Db, frag_db: DBFrag<Db::ReadOnly>, runtime: Arc<Runtime>, config: SequencerConfig) -> Self {
        Self {
            data: SharedData {
                db,
                frag_db,
                runtime,
                config,
                tx_pool: Default::default(),
                fork_choice_state: Default::default(),
                payload_attributes: Default::default(),
            },
            state: Default::default(),
        }
    }
}

const DEFAULT_BASE_FEE: u64 = 10;

impl<Db: BopDB> Actor<Db::ReadOnly> for Sequencer<Db> {
    const CORE_AFFINITY: Option<usize> = Some(0);

    fn loop_body(&mut self, connections: &mut Connections<SendersSpine<Db::ReadOnly>, ReceiversSpine<Db::ReadOnly>>) {
        connections.receive(|msg, senders| {
            self.state =
                std::mem::take(&mut self.state).update(SequencerEvent::SimResult(msg), &mut self.data, senders);
        });

        connections.receive(|msg, senders| {
            self.handle_engine_api_message(msg, senders);
        });

        connections.receive(|msg, senders| {
            self.state = std::mem::take(&mut self.state).update(SequencerEvent::NewTx(msg), &mut self.data, senders);
        });

        connections.receive(|msg, _| {
            // Process blocks as they arrive
            self.handle_block(msg);
        });
    }
}

impl<Db: BopDB> Sequencer<Db> {
    fn handle_block(&mut self, block_result: Result<BlockWithSenders<Block<OpTransactionSigned>>, reqwest::Error>) {
        let block = block_result.expect("failed to fetch block");
        todo!()
        // if block.header.number == payload_block_number - 1 {
        //     //TODO: switch in or out of block sync state
        // }
    }

    /// Handles messages from the engine API.
    ///
    /// - `NewPayloadV3` triggers a block sync if the payload is for a new block.
    fn handle_engine_api_message(&self, msg: messages::EngineApi, senders: &SendersSpine<Db::ReadOnly>) {
        match msg {
            messages::EngineApi::NewPayloadV3 {
                payload,
                versioned_hashes: _,
                parent_beacon_block_root: _,
                res_tx: _,
            } => {
                let seq_block_number = payload.payload_inner.payload_inner.block_number; // TODO: this should be accessible from the DB
                let payload_block_number = payload.payload_inner.payload_inner.block_number;

                if payload_block_number <= seq_block_number {
                    tracing::debug!(
                        "ignoring old payload for block {} because sequencer is at {}",
                        payload_block_number,
                        seq_block_number
                    );
                    return;
                }

                if payload_block_number > seq_block_number + 1 {
                    tracing::info!(
                        "sequencer is behind, fetching blocks from {} to {}",
                        seq_block_number + 1,
                        payload_block_number
                    );

                    fetch_blocks_and_send_sequentially(
                        seq_block_number + 1,
                        payload_block_number - 1,
                        "TODO".to_string(),
                        senders.into(),
                        &self.data.runtime,
                    );
                }

                // TODO: apply new payload
            }
            messages::EngineApi::ForkChoiceUpdatedV3 { fork_choice_state: _, payload_attributes: _, res_tx: _ } => {}
            messages::EngineApi::GetPayloadV3 { payload_id: _, res: _ } => {}
        }
    }
}
