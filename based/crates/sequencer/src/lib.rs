use std::{collections::VecDeque, sync::Arc};

use alloy_consensus::Header;
use alloy_rpc_types::engine::{CancunPayloadFields, ExecutionPayload, ExecutionPayloadSidecar, ForkchoiceState};
use block_sync::BlockSync;
use bop_common::{
    actor::Actor,
    communication::{
        messages::{self, BlockSyncMessage, EngineApi, SequencerToSimulator, SimulatorToSequencer},
        Connections, ReceiversSpine, SendersSpine, SpineConnections, TrackedSenders,
    },
    db::{BopDB, DBFrag},
    p2p::VersionedMessage,
    time::{Duration, Instant},
    transaction::Transaction,
};
use bop_pool::transaction::pool::TxPool;
use frag::FragSequence;
use op_alloy_rpc_types_engine::OpPayloadAttributes;
use reqwest::Url;
use reth_evm::{ConfigureEvmEnv, NextBlockEnvAttributes};
use reth_optimism_chainspec::OpChainSpecBuilder;
use reth_optimism_evm::OpEvmConfig;
use revm_primitives::{Address, BlockEnv, B256};
use strum_macros::AsRefStr;
use tokio::runtime::Runtime;
use tracing::{error, warn};

pub mod block_sync;
mod frag;
mod sorting;

use sorting::{ActiveOrders, SortingData};

#[derive(Clone, Debug, Default, AsRefStr)]
pub enum SequencerState<Db: BopDB> {
    /// Synced and waiting for the next new payload message
    #[default]
    WaitingForNewPayload,
    /// Waiting for fork choice without attributes
    WaitingForForkChoice(ExecutionPayload, ExecutionPayloadSidecar),
    /// Waiting for fork choice with attributes
    WaitingForAttributes,
    /// Building frags and blocks
    Sorting(SortingData<Db::ReadOnly>),
    /// Waiting for block sync
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
        data: &mut SequencerContext<Db>,
        senders: &SendersSpine<Db::ReadOnly>,
    ) -> SequencerState<Db> {
        use EngineApi::*;
        use SequencerState::*;

        match (msg, self) {
            (NewPayloadV3 { .. }, Syncing { last_block_number }) => Syncing { last_block_number },

            (
                NewPayloadV3 { payload, versioned_hashes, parent_beacon_block_root, .. },
                WaitingForNewPayload | WaitingForAttributes,
            ) => WaitingForForkChoice(
                ExecutionPayload::V3(payload),
                ExecutionPayloadSidecar::v3(CancunPayloadFields::new(parent_beacon_block_root, versioned_hashes)),
            ),

            (ForkChoiceUpdatedV3 { payload_attributes: None, .. }, WaitingForForkChoice(payload, sidecar)) => {
                // clear all the temp state on the db
                data.frags.clear_frags();

                if let Some(last_block_number) = data
                    .block_executor
                    .apply_new_payload(payload, sidecar, &data.db, senders, true)
                    .expect("Issue with block sync")
                {
                    Syncing { last_block_number }
                } else {
                    WaitingForAttributes
                }
            }

            (ForkChoiceUpdatedV3 { payload_attributes: Some(attributes), .. }, WaitingForAttributes) => {
                // start building

                let next_attr = NextBlockEnvAttributes {
                    timestamp: attributes.payload_attributes.timestamp,
                    suggested_fee_recipient: data.config.coinbase,
                    prev_randao: attributes.payload_attributes.prev_randao,
                    gas_limit: data.parent_header.gas_limit,
                };

                let block_env = data
                    .config
                    .evm_config
                    .next_cfg_and_block_env(&data.parent_header, next_attr.clone())
                    .expect("couldn't create blockenv");
                // should never fail as its a broadcast
                senders
                    .send_timeout(block_env.block_env.clone(), Duration::from_millis(10))
                    .expect("couldn't send block env");
                data.block_env = block_env.block_env;

                let txs = attributes
                    .transactions
                    .map(|txs| txs.into_iter().map(|bytes| Transaction::decode(bytes).unwrap().into()).collect())
                    .unwrap_or_default();

                let sorting_data = data.new_sorting_data(txs, !attributes.no_tx_pool.unwrap_or(false));
                Sorting(sorting_data)
            }

            (GetPayloadV3 { res, .. }, Self::Sorting(sorting_data)) => {
                let mut frag = data.frags.apply_sorted_frag(sorting_data.frag);
                frag.is_last = true;
                let frag_msg = VersionedMessage::from(frag);
                // gossip seal to p2p
                let _ = senders.send(frag_msg);

                let (seal, block) =
                    data.frags.seal_block(&data.block_env, &data.config.evm_config.chain_spec(), data.parent_hash);

                // gossip seal to p2p
                let _ = senders.send(VersionedMessage::from(seal));

                // send payload back to rpc
                let _ = res.send(block);
                // now we should our payload back from the node
                WaitingForNewPayload
            }

            // fallback
            (m, s) => {
                debug_assert!(false, "don't know how to handle msg {m:?} in state {}", s.as_ref());
                s
            }
        }
    }

    fn handle_block_sync(self, block: BlockSyncMessage, data: &mut SequencerContext<Db>) -> Self {
        use SequencerState::*;

        let Ok(block) = block else {
            todo!("handle block sync error");
        };

        match self {
            Syncing { last_block_number } => {
                data.block_executor.apply_and_commit_block(&block, &data.db, true).expect("issue syncing block");

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

    fn handle_new_tx(
        self,
        tx: Arc<Transaction>,
        data: &mut SequencerContext<Db>,
        senders: &SendersSpine<Db::ReadOnly>,
    ) -> Self {
        data.tx_pool.handle_new_tx(tx, data.frags.db_ref(), data.base_fee, senders);
        self
    }

    fn handle_sim_result(
        self,
        result: SimulatorToSequencer<Db::ReadOnly>,
        data: &mut SequencerContext<Db>,
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
                if sort_data.is_valid(result.state_id) {
                    warn!("received sim result on wrong state, dropping");
                    return SequencerState::Sorting(sort_data);
                }
                sort_data.handle_sim(simulated_tx, sender, data.base_fee);
                SequencerState::Sorting(sort_data)
            }

            TxTof(simulated_tx) => {
                match simulated_tx {
                    Ok(res) if data.frags.is_valid(result.state_id) => data.tx_pool.handle_simulated(res),

                    // resend because was on the wrong hash
                    Ok(res) => {
                        let _ = senders.send_timeout(
                            SequencerToSimulator::SimulateTxTof(res.tx, data.frags.db()),
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

    /// Checks at every loop:
    /// - if enough time has passed, seal the current frag and go to the next one
    /// - if all sims are done, send next batch of sims
    fn tick(self, data: &mut SequencerContext<Db>, connections: &mut SpineConnections<Db::ReadOnly>) -> Self {
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

    pub fn update(
        self,
        event: SequencerEvent<Db>,
        data: &mut SequencerContext<Db>,
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
    coinbase: Address,
}
impl Default for SequencerConfig {
    fn default() -> Self {
        let chainspec = Arc::new(OpChainSpecBuilder::base_mainnet().build());
        let evm_config = OpEvmConfig::new(chainspec);

        Self {
            frag_duration: Duration::from_millis(200),
            max_gas: 300_000_000,
            n_per_loop: 10,
            rpc_url: Url::parse("http://0.0.0.0:8003").unwrap(),
            evm_config,
            coinbase: Address::random(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct SequencerContext<Db: BopDB> {
    config: SequencerConfig,
    db: Db,
    tx_pool: TxPool,
    block_env: BlockEnv,
    frags: FragSequence<Db::ReadOnly>,
    block_executor: BlockSync,
    parent_hash: B256,
    parent_header: Header,
    _fork_choice_state: ForkchoiceState,
    _payload_attributes: Box<OpPayloadAttributes>,
    //TODO: set from blocksync
    base_fee: u64,
}

impl<Db: BopDB> SequencerContext<Db> {
    fn new_sorting_data(
        &self,
        remaining_attributes_txs: VecDeque<Arc<Transaction>>,
        can_add_txs: bool,
    ) -> SortingData<Db::ReadOnly> {
        SortingData {
            frag: self.frags.create_in_sort(),
            until: Instant::now() + self.config.frag_duration,
            in_flight_sims: 0,
            next_to_be_applied: None,
            tof_snapshot: ActiveOrders::new(self.tx_pool.clone_active()),
            remaining_attributes_txs,
            can_add_txs,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Sequencer<Db: BopDB> {
    state: SequencerState<Db>,
    data: SequencerContext<Db>,
}

impl<Db: BopDB> Sequencer<Db> {
    pub fn new(db: Db, frag_db: DBFrag<Db::ReadOnly>, runtime: Arc<Runtime>, config: SequencerConfig) -> Self {
        let frags = FragSequence::new(frag_db, config.max_gas);
        let block_executor = BlockSync::new(
            Arc::new(OpChainSpecBuilder::base_mainnet().build()),
            runtime.into(),
            config.rpc_url.clone(),
        );

        Self {
            state: SequencerState::default(),

            data: SequencerContext {
                db,
                frags,
                block_executor,
                config,
                tx_pool: Default::default(),
                _fork_choice_state: Default::default(),
                _payload_attributes: Default::default(),
                base_fee: Default::default(),
                parent_hash: Default::default(),
                parent_header: Default::default(),
                block_env: Default::default(),
            },
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

        // checks on every loop:
        // - seal current frag
        // - send new batch of sims
        self.state = std::mem::take(&mut self.state).tick(&mut self.data, connections);
    }
}
