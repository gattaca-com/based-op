use std::{fmt::Display, ops::Deref, sync::Arc};

use alloy_consensus::Block;
use alloy_rpc_types::engine::ForkchoiceState;
use block_sync::BlockSync;
use bop_common::{
    actor::Actor,
    communication::{
        messages::{self, BlockSyncMessage, SequencerToSimulator, SimulationError, SimulatorToSequencer},
        Connections, ReceiversSpine, SendersSpine, TrackedSenders,
    },
    db::{BopDB, BopDbRead, DBFrag, DBSorting},
    time::{Duration, Instant},
    transaction::{SimulatedTx, SimulatedTxList, Transaction},
};
use bop_pool::transaction::pool::TxPool;
use built_frag::BuiltFrag;
use op_alloy_rpc_types_engine::OpPayloadAttributes;
use reqwest::Url;
use reth_evm::execute::ProviderError;
use reth_optimism_chainspec::OpChainSpecBuilder;
use revm::Database;
use revm_primitives::{Address, B256};
use strum_macros::AsRefStr;
use tokio::runtime::Runtime;
use tracing::{error, warn};

pub(crate) mod block_sync;
pub(crate) mod built_frag;
pub(crate) mod sorting;
use sorting::SortingData;

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
    BlockSync(BlockSyncMessage),
    NewTx(Arc<Transaction>),
    SimResult(SimulatorToSequencer<Db>),
    EngineApi(messages::EngineApi),
}

impl<Db> SequencerState<Db>
where
    Db: BopDB + BopDbRead + Database<Error: Into<ProviderError> + Display>,
{
    pub fn start_sorting(data: &SharedData<Db>) -> Self {
        Self::Sorting(data.into())
    }

    fn handle_block_sync(
        self,
        block: BlockSyncMessage,
        data: &mut SharedData<Db>,
        senders: &SendersSpine<Db::ReadOnly>,
    ) -> Self {
        let Ok(block) = block else {
            todo!("handle block sync error");
        };
        use SequencerState::*;
        match self {
            WaitingForSync | WaitingForPayloadAttributes => {
                data.block_executor.apply_and_commit_block(&block, &data.db).expect("issue syncing block");
                //TODO: handle payload before blocksync
                WaitingForPayloadAttributes
            }
            Syncing { last_block_number } => {
                data.block_executor.apply_and_commit_block(&block, &data.db).expect("issue syncing block");
                if block.number != last_block_number {
                    return Syncing { last_block_number };
                }
                //TODO: refetch to check whether it's actually still the last
                let new_last = data.block_executor.last_block_number();
                if new_last == last_block_number {
                    return WaitingForPayloadAttributes;
                } else {
                    data.block_executor.fetch_until_last(last_block_number, senders).expect("issue syncing block");
                    return Syncing { last_block_number: new_last };
                }
            }
            _ => {
                debug_assert!(false, "Should not have received block sync update while in state {self:?}");
                self
            }
        }
    }

    fn handle_new_tx(
        self,
        tx: Arc<Transaction>,
        data: &mut SharedData<Db>,
        senders: &SendersSpine<Db::ReadOnly>,
    ) -> Self {
        data.tx_pool.handle_new_tx(tx, &data.frag_db, data.base_fee, senders);
        self
    }

    fn handle_sim_result(
        self,
        result: SimulatorToSequencer<Db::ReadOnly>,
        data: &mut SharedData<Db>,
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
                if sort_data.is_valid(result.unique_hash) {
                    warn!("received sim result on wrong state, dropping");
                    return SequencerState::Sorting(sort_data);
                }
                sort_data.handle_sim(simulated_tx, sender, data.base_fee);
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

    fn _update(self, data: &mut SharedData<Db>, senders: &SendersSpine<Db::ReadOnly>) -> Self {
        use SequencerState::*;
        match self {
            Sorting(sorting_data) if sorting_data.should_frag() => {
                todo!("Seal and send frag")
            }
            Sorting(sorting_data) if sorting_data.finished_iteration() => {
                Sorting(sorting_data.apply_and_send_next(data.config.n_per_loop, senders, data.base_fee))
            }
            _ => self,
        }
    }

    fn handle_engine_api(
        self,
        msg: messages::EngineApi,
        data: &mut SharedData<Db>,
        senders: &SendersSpine<<Db as BopDB>::ReadOnly>,
    ) -> SequencerState<Db> {
        todo!()
    }

    pub fn update(
        self,
        event: SequencerEvent<Db::ReadOnly>,
        data: &mut SharedData<Db>,
        senders: &SendersSpine<Db::ReadOnly>,
    ) -> Self {
        use SequencerEvent::*;
        match event {
            BlockSync(block) => self.handle_block_sync(block, data, senders),
            NewTx(tx) => self.handle_new_tx(tx, data, senders),
            SimResult(res) => self.handle_sim_result(res, data, senders),
            EngineApi(msg) => self.handle_engine_api(msg, data, senders),
        }
        ._update(data, senders)
    }
}

#[derive(Clone, Debug)]
pub struct SequencerConfig {
    frag_duration: Duration,
    max_gas: u64,
    n_per_loop: usize,
    rpc_url: Url,
}
impl Default for SequencerConfig {
    fn default() -> Self {
        Self { frag_duration: Duration::from_millis(200), max_gas: 300_000_000, n_per_loop: 10, rpc_url: todo!() }
    }
}

#[derive(Clone, Debug)]
pub struct SharedData<Db: BopDB + BopDbRead> {
    tx_pool: TxPool,
    db: Db,
    frag_db: DBFrag<Db::ReadOnly>,
    block_executor: BlockSync,
    config: SequencerConfig,
    fork_choice_state: ForkchoiceState,
    payload_attributes: Box<OpPayloadAttributes>,
    //TODO: set from blocksync
    base_fee: u64,
}

#[derive(Clone, Debug)]
pub struct Sequencer<Db: BopDB + BopDbRead> {
    state: SequencerState<Db>,
    data: SharedData<Db>,
}

impl<Db: BopDB + BopDbRead> Sequencer<Db> {
    pub fn new(db: Db, frag_db: DBFrag<Db::ReadOnly>, runtime: Arc<Runtime>, config: SequencerConfig) -> Self {
        Self {
            data: SharedData {
                db,
                frag_db,
                block_executor: BlockSync::new(
                    Arc::new(OpChainSpecBuilder::base_mainnet().build()),
                    runtime,
                    config.rpc_url.clone(),
                ),
                config,
                tx_pool: Default::default(),
                fork_choice_state: Default::default(),
                payload_attributes: Default::default(),
                base_fee: Default::default(),
            },
            state: Default::default(),
        }
    }
}

impl<Db> Actor<Db::ReadOnly> for Sequencer<Db>
where
    Db: BopDB + BopDbRead + Database<Error: Into<ProviderError> + Display>,
{
    const CORE_AFFINITY: Option<usize> = Some(0);

    fn loop_body(&mut self, connections: &mut Connections<SendersSpine<Db::ReadOnly>, ReceiversSpine<Db::ReadOnly>>) {
        connections.receive(|msg, senders| {
            self.state =
                std::mem::take(&mut self.state).update(SequencerEvent::SimResult(msg), &mut self.data, senders);
        });

        connections.receive(|msg, senders| {
            self.state =
                std::mem::take(&mut self.state).update(SequencerEvent::EngineApi(msg), &mut self.data, senders);
        });

        connections.receive(|msg, senders| {
            self.state = std::mem::take(&mut self.state).update(SequencerEvent::NewTx(msg), &mut self.data, senders);
        });

        connections.receive(|msg, senders| {
            // Process blocks as they arrive
            self.state =
                std::mem::take(&mut self.state).update(SequencerEvent::BlockSync(msg), &mut self.data, senders);
        });
    }
}
