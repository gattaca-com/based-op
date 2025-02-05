use std::sync::Arc;

use alloy_rpc_types::engine::{CancunPayloadFields, ExecutionPayload, ExecutionPayloadSidecar};
use block_sync::BlockSync;
use bop_common::{
    actor::Actor,
    communication::{
        messages::{self, BlockFetch, BlockSyncError, BlockSyncMessage, EngineApi, SequencerToSimulator, SimulatorToSequencer},
        Connections, ReceiversSpine, SendersSpine, SpineConnections, TrackedSenders,
    },
    db::{DatabaseWrite, DBFrag},
    p2p::VersionedMessage,
    time::Duration,
    transaction::Transaction,
};
use bop_db::DatabaseRead;
use bop_pool::transaction::pool::TxPool;
use frag::FragSequence;
use reth_evm::{ConfigureEvmEnv, NextBlockEnvAttributes};
use reth_optimism_chainspec::{OpChainSpec, OpChainSpecBuilder};
use reth_optimism_evm::OpEvmConfig;
use reth_optimism_primitives::OpTransactionSigned;
use reth_primitives::BlockWithSenders;
use reth_primitives_traits::SignedTransaction;
use revm_primitives::{Address, BlockEnv, B256};
use strum_macros::AsRefStr;
use tokio::runtime::Runtime;
use tracing::{error, warn};

pub mod block_sync;
pub mod config;
mod context;
mod frag;
mod sorting;

pub use config::SequencerConfig;
use context::SequencerContext;
use sorting::SortingData;

pub fn payload_to_block(payload: ExecutionPayload, sidecar: ExecutionPayloadSidecar) -> Result<BlockSyncMessage, BlockSyncError> {
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

#[derive(Clone, Debug, Default, AsRefStr)]
pub enum SequencerState<Db: DatabaseWrite + DatabaseRead> {
    /// Synced and waiting for the next new payload message
    #[default]
    WaitingForNewPayload,
    /// Waiting for fork choice without attributes
    WaitingForForkChoice(ExecutionPayload, ExecutionPayloadSidecar),
    /// Waiting for fork choice with attributes
    WaitingForAttributes,
    /// Building frags and blocks
    Sorting(SortingData<Db>),
    /// Waiting for block sync
    Syncing {
        /// When the stage reaches this syncing is done
        /// TODO: not always true as we may have fallen behind the head - we should cache all payloads that come in while syncing
        last_block_number: u64,
    },
}

#[derive(Debug, AsRefStr)]
#[repr(u8)]
pub enum SequencerEvent<Db: DatabaseWrite + DatabaseRead> {
    BlockSync(BlockSyncMessage),
    NewTx(Arc<Transaction>),
    SimResult(SimulatorToSequencer<Db>),
    EngineApi(EngineApi),
}

impl<Db> SequencerState<Db>
where
    Db: DatabaseWrite + DatabaseRead,
{
    /// Engine API messages signify a change in the Sequencer's state.
    /// 
    /// The only case where we don't change state is if we are in the Syncing state.
    /// While syncing, we cache all payloads that come in.
    /// 
    /// Once synced we trigger sync through
    /// 
    fn handle_engine_api(
        self,
        msg: EngineApi,
        data: &mut SequencerContext<Db>,
        senders: &SendersSpine<Db>,
    ) -> SequencerState<Db> {
        use EngineApi::*;
        use SequencerState::*;

        match (msg, self) {
            (
                NewPayloadV3 { payload, versioned_hashes, parent_beacon_block_root, .. },
                Syncing { last_block_number },
            ) => {
                let payload = ExecutionPayload::V3(payload);
                let bn = payload.block_number();
                let sidecar =
                    ExecutionPayloadSidecar::v3(CancunPayloadFields::new(parent_beacon_block_root, versioned_hashes));
                senders.send(BlockFetch::FromPayload(payload, sidecar));
                Syncing { last_block_number: bn }
            }

            (
                NewPayloadV3 { payload, versioned_hashes, parent_beacon_block_root, .. },
                WaitingForNewPayload | WaitingForAttributes,
            ) => WaitingForForkChoice(
                ExecutionPayload::V3(payload),
                ExecutionPayloadSidecar::v3(CancunPayloadFields::new(parent_beacon_block_root, versioned_hashes)),
            ),

            (ForkChoiceUpdatedV3 { payload_attributes: None, .. }, WaitingForForkChoice(payload, sidecar)) => {
                let head_bn = data.db.head_block_number().expect("couldn't get db");
                if payload.block_number() > head_bn + 1 {
                    let last_block_number = payload.block_number() - 1;
                    senders.send(BlockFetch::FromTo(head_bn + 1, last_block_number));
                    senders.send(BlockFetch::FromPayload(payload, sidecar));
                    Syncing { last_block_number }
                } else {
                    let block = payload_to_block(payload, sidecar).expect("couldn't get block from payload");
                    data.block_executor.apply_and_commit_block(&block, &data.db, true);
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
                    .next_cfg_and_block_env(&data.parent_header, next_attr)
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
                    data.frags.seal_block(&data.block_env, data.config.evm_config.chain_spec(), data.parent_hash);

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

    fn handle_new_tx(
        self,
        tx: Arc<Transaction>,
        data: &mut SequencerContext<Db>,
        senders: &SendersSpine<Db>,
    ) -> Self {
        data.tx_pool.handle_new_tx(tx, data.frags.db_ref(), data.base_fee, senders);
        self
    }

    fn handle_sim_result(
        self,
        result: SimulatorToSequencer<Db>,
        data: &mut SequencerContext<Db>,
        senders: &SendersSpine<Db>,
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

    pub fn update(
        self,
        event: SequencerEvent<Db>,
        data: &mut SequencerContext<Db>,
        senders: &SendersSpine<Db>,
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
pub struct Sequencer<Db: DatabaseWrite + DatabaseRead> {
    state: SequencerState<Db>,
    data: SequencerContext<Db>,
}

impl<Db: DatabaseWrite + DatabaseRead> Sequencer<Db> {
    pub fn new(db: Db, frag_db: DBFrag<Db>, config: SequencerConfig) -> Self {
        let frags = FragSequence::new(frag_db, config.max_gas);
        let block_executor =
            BlockSync::new(config.evm_config.chain_spec().clone(), config.rpc_url.clone());

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
        // handle sim results
        connections.receive(|msg, senders| {
            self.state =
                std::mem::take(&mut self.state).update(SequencerEvent::SimResult(msg), &mut self.data, senders);
        });

        // handle engine API messages from rpc
        connections.receive(|msg, senders| {
            tracing::info!("got engine api msg {msg:?}");
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
