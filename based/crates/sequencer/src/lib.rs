use std::sync::Arc;

use alloy_consensus::Header;
use alloy_rpc_types::engine::{CancunPayloadFields, ExecutionPayload, ExecutionPayloadSidecar, ForkchoiceState};
use block_sync::BlockSync;
use bop_common::{
    actor::Actor,
    communication::{
        messages::{self, BlockSyncMessage, EngineApi, SequencerToSimulator, SimulationError, SimulatorToSequencer},
        Connections, ReceiversSpine, SendersSpine, SpineConnections,TrackedSenders,
    },
    db::{BopDB, BopDbRead, DBFrag},
    p2p::FragMessage,
    time::{Duration, Instant, Repeater},
    transaction::{SimulatedTx, SimulatedTxList, Transaction},
};
use bop_pool::transaction::pool::TxPool;
use op_alloy_rpc_types_engine::OpPayloadAttributes;
use reqwest::Url;
use reth_evm::{ConfigureEvmEnv, NextBlockEnvAttributes};
use reth_optimism_chainspec::OpChainSpecBuilder;
use reth_optimism_evm::OpEvmConfig;
use revm_primitives::{BlockEnv, B256};
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
    /// This holds the previous payload to be applied
    WaitingForPayloadAttributes(ExecutionPayload, ExecutionPayloadSidecar),
    Sorting(SortingData<Db::ReadOnly>),
    Syncing {
        /// When the stage reaches this syncing is done
        last_block_number: u64,
    },
}

#[derive(Debug, AsRefStr)]
#[repr(u8)]
pub enum SequencerEvent<Db: BopDB> {
    BlockSync(BlockSyncMessage),
    NewTx(Arc<Transaction>),
    SimResult(SimulatorToSequencer<Db::ReadOnly>),
    EngineApi(EngineApi),
}

impl<Db> SequencerState<Db>
where
    Db: BopDB,
{
    fn handle_engine_api(
        self,
        msg: EngineApi,
        data: &mut SharedData<Db>,
        senders: &SendersSpine<Db::ReadOnly>,
    ) -> SequencerState<Db> {
        match (msg, self) {
            (
                EngineApi::NewPayloadV3 { payload, versioned_hashes, parent_beacon_block_root, .. },
                SequencerState::Syncing { last_block_number },
            ) => {
                let last_block_number = data
                    .block_executor
                    .apply_new_payload(
                        ExecutionPayload::V3(payload),
                        ExecutionPayloadSidecar::v3(CancunPayloadFields::new(
                            parent_beacon_block_root,
                            versioned_hashes,
                        )),
                        &data.db,
                        Some(last_block_number),
                        senders,
                    )
                    .expect("Issue with block sync")
                    .expect("should have gotten a next last block number");
                SequencerState::Syncing { last_block_number }
            }
            (
                EngineApi::NewPayloadV3 { payload, versioned_hashes, parent_beacon_block_root, .. },
                SequencerState::WaitingForSync,
            ) => SequencerState::WaitingForPayloadAttributes(
                ExecutionPayload::V3(payload),
                ExecutionPayloadSidecar::v3(CancunPayloadFields::new(parent_beacon_block_root, versioned_hashes)),
            ),

            (
                EngineApi::ForkChoiceUpdatedV3 { payload_attributes, res_tx, .. },
                SequencerState::WaitingForPayloadAttributes(payload, sidecar),
            ) => {
                if let Some(last_block_number) = data
                    .block_executor
                    .apply_new_payload(payload, sidecar, &data.db, None, senders)
                    .expect("Issue with block sync")
                {
                    SequencerState::Syncing { last_block_number }
                } else if let Some(attributes) = payload_attributes {
                    // data.payload_attributes = attributes;
                    data.create_and_apply_first_frag();
                    let next_attr = NextBlockEnvAttributes {
                        timestamp: attributes.payload_attributes.timestamp,
                        suggested_fee_recipient: attributes.payload_attributes.suggested_fee_recipient,
                        prev_randao: attributes.payload_attributes.prev_randao,
                        gas_limit: data.parent_header.gas_limit,
                    };

                    let block_env = data
                        .config
                        .evm_config
                        .next_cfg_and_block_env(&data.parent_header, next_attr)
                        .expect("couldn't create blockenv");
                    // should never fail as its a broadcast
                    senders
                        .send_timeout(block_env.block_env, Duration::from_millis(10))
                        .expect("couldn't send block env");
                    SequencerState::Sorting(SortingData::from(&(*data)))
                } else {
                    SequencerState::WaitingForSync
                }
            }
            (EngineApi::GetPayloadV3 { payload_id, res }, Self::Sorting(sorting_data)) => {
                todo!();
                //sorting_data.commit_frag();
                //res.send(data.build_block(payload_id))
            }
            (m, s) => {
                debug_assert!(false, "don't know how to handle msg {m:?} in state {}", s.as_ref());
                s
            }
        }
    }

    fn handle_block_sync(self, block: BlockSyncMessage, data: &mut SharedData<Db>) -> Self {
        let Ok(block) = block else {
            todo!("handle block sync error");
        };
        use SequencerState::*;
        match self {
            Syncing { last_block_number } => {
                data.block_executor.apply_and_commit_block(&block, &data.db).expect("issue syncing block");
                if block.number != last_block_number {
                    Syncing { last_block_number }
                } else {
                    // Wait until the next payload and attributes arrive
                    WaitingForSync
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

    fn tick(self, data: &mut SharedData<Db>, connections: &mut SpineConnections<Db::ReadOnly>) -> Self {
        use SequencerState::*;
        match self {
            Sorting(sorting_data) if sorting_data.should_frag() => {
                let frag = sorting_data.get_frag();
                let _ = connections.send(frag);

                Sorting(sorting_data.apply_and_send_next(data.config.n_per_loop, connections, data.base_fee))
            }
            Sorting(sorting_data) if sorting_data.finished_iteration() => {
                Sorting(sorting_data.apply_and_send_next(data.config.n_per_loop, connections, data.base_fee))
            }
            _ => self,
        }
    }

    pub fn update(
        self,
        event: SequencerEvent<Db>,
        data: &mut SharedData<Db>,
        senders: &SendersSpine<Db::ReadOnly>,
    ) -> Self {
        use SequencerEvent::*;
        match event {
            BlockSync(block) => self.handle_block_sync(block, data),
            NewTx(tx) => self.handle_new_tx(tx, data, senders),
            SimResult(res) => self.handle_sim_result(res, data, senders),
            EngineApi(msg) => self.handle_engine_api(msg, data, senders),
        }
    }
}

#[derive(Clone, Debug)]
pub struct SequencerConfig {
    frag_duration: Duration,
    max_gas: u64,
    n_per_loop: usize,
    rpc_url: Url,
    evm_config: OpEvmConfig,
}
impl Default for SequencerConfig {
    fn default() -> Self {
        let chainspec = Arc::new(OpChainSpecBuilder::base_mainnet().build());
        let evm_config = OpEvmConfig::new(chainspec);

        Self {
            frag_duration: Duration::from_millis(200),
            max_gas: 300_000_000,
            n_per_loop: 10,
            rpc_url: Url::parse("0.0.0.0:8003").unwrap(),
            evm_config,
        }
    }
}

#[derive(Clone, Debug)]
pub struct SharedData<Db: BopDB> {
    tx_pool: TxPool,
    db: Db,
    /// DB with the frags applied on the last top of block
    frag_db: DBFrag<Db::ReadOnly>,
    block_executor: BlockSync,
    config: SequencerConfig,
    parent_hash: B256,
    parent_header: Header,
    fork_choice_state: ForkchoiceState,

    payload_attributes: Box<OpPayloadAttributes>,
    //TODO: set from blocksync
    base_fee: u64,
}
impl<Db: BopDB> SharedData<Db> {
    fn create_and_apply_first_frag(&self) {
        todo!()
    }
}

#[derive(Clone, Debug)]
pub struct Sequencer<Db: BopDB> {
    state: SequencerState<Db>,
    data: SharedData<Db>,
    repeater: Repeater
}

impl<Db: BopDB> Sequencer<Db> {
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
                parent_hash: Default::default(),
                parent_header: Default::default(),
            },
            state: Default::default(),
            repeater: Repeater::every(Duration::from_secs(1))
        }
    }
}

impl<Db> Actor<Db::ReadOnly> for Sequencer<Db>
where
    Db: BopDB,
{
    const CORE_AFFINITY: Option<usize> = Some(0);

    fn loop_body(&mut self, connections: &mut Connections<SendersSpine<Db::ReadOnly>, ReceiversSpine<Db::ReadOnly>>) {
        // handle sim results
        connections.receive(|msg, senders| {
            self.state =
                std::mem::take(&mut self.state).update(SequencerEvent::SimResult(msg), &mut self.data, senders);
        });

        // handle engine API messages from rpc
        connections.receive(|msg, senders| {
            self.state =
                std::mem::take(&mut self.state).update(SequencerEvent::EngineApi(msg), &mut self.data, senders);
        });

        // handle new transaction from rpc
        connections.receive(|msg, senders| {
            self.state = std::mem::take(&mut self.state).update(SequencerEvent::NewTx(msg), &mut self.data, senders);
        });

        // handle block sync
        connections.receive(|msg, senders| {
            self.state =
                std::mem::take(&mut self.state).update(SequencerEvent::BlockSync(msg), &mut self.data, senders);
        });

        // tick checks on every loop
        self.state = std::mem::take(&mut self.state).tick(&mut self.data, connections)
        if self.repeater.fired() {
            connections.send(BlockEnv::default());
        }
    }
}
