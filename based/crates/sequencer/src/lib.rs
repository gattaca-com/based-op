use std::sync::Arc;

use alloy_consensus::Block;
use alloy_rpc_types::engine::ForkchoiceState;
use bop_common::{
    actor::Actor,
    communication::{
        messages::{self, SimulatorToSequencer},
        Connections, ReceiversSpine, SendersSpine,
    },
    time::{Duration, Instant},
    transaction::Transaction,
};
use bop_common::db::{BopDB, BopDbRead, DBFrag, DBSorting};
use bop_pool::transaction::pool::TxPool;
use built_block::BuiltBlock;
use op_alloy_rpc_types_engine::OpPayloadAttributes;
use reth_optimism_node::OpPayloadBuilderAttributes;
use reth_optimism_primitives::OpTransactionSigned;
use reth_primitives::BlockWithSenders;
use revm_primitives::db::DatabaseRef;
use strum_macros::AsRefStr;
use tokio::runtime::Runtime;
use tracing::info;

use crate::block_sync::fetch_blocks::fetch_blocks_and_send_sequentially;

pub(crate) mod block_sync;
pub(crate) mod built_block;

#[derive(Clone, Debug, Default, AsRefStr)]
pub enum SequencerState<DbRead> {
    #[default]
    WaitingForSync,
    WaitingForPayloadAttributes,
    Sorting {
        /// This is the db that is built on top of the last block chunk to be used to
        /// build a new cachedb on top of for sorting
        /// starting a new sort
        db: DBFrag<DbRead>,
        block: BuiltBlock,
        until: Instant,
    },
    Syncing {
        /// When the stage reaches this syncing is done
        last_block_number: u64,
    },
}

impl<DbRead> SequencerState<DbRead> {
    pub fn update<Db:BopDB>(
        mut self,
        event: SequencerEvent,
        shared_data: &mut SharedData<Db, DbRead>,
        senders: &SendersSpine<Db>,
    ) -> Self {
        use SequencerEvent::*;
        use SequencerState::*;
        match (event, self) {
            (BlockSync(block_with_senders), WaitingForSync) => todo!(),
            (BlockSync(block_with_senders), WaitingForPayloadAttributes) => todo!(),
            (BlockSync(block_with_senders), Sorting { db, block, until }) => todo!(),
            (BlockSync(block_with_senders), Syncing { last_block_number }) => todo!(),
            (ReceivedPayloadAttribues(op_payload_builder_attributes), WaitingForSync) => todo!(),
            (ReceivedPayloadAttribues(op_payload_builder_attributes), WaitingForPayloadAttributes) => todo!(),
            (ReceivedPayloadAttribues(op_payload_builder_attributes), Sorting { db, block, until }) => todo!(),
            (ReceivedPayloadAttribues(op_payload_builder_attributes), Syncing { last_block_number }) => todo!(),
            (NewTx(arc), WaitingForSync) => todo!(),
            (NewTx(arc), WaitingForPayloadAttributes) => todo!(),
            (NewTx(arc), Sorting { db, block, until }) => todo!(),
            (NewTx(arc), Syncing { last_block_number }) => todo!(),
        }
    }
}

#[derive(Debug, AsRefStr)]
#[repr(u8)]
pub enum SequencerEvent {
    BlockSync(Result<BlockWithSenders<Block<OpTransactionSigned>>, reqwest::Error>),
    ReceivedPayloadAttribues(Option<Box<OpPayloadBuilderAttributes>>),
    NewTx(Arc<Transaction>),
}

#[derive(Clone, Copy, Debug)]
pub struct SequencerConfig {
    frag_duration: Duration,
}
impl Default for SequencerConfig {
    fn default() -> Self {
        Self { frag_duration: Duration::from_millis(200) }
    }
}

#[derive(Clone, Debug)]
pub struct SharedData<Db, DbRead> {
    tx_pool: TxPool,
    db: Db,
    frag_db: DBFrag<DbRead>,
    runtime: Arc<Runtime>,
    config: SequencerConfig,
    fork_choice_state: ForkchoiceState,
    payload_attributes: Box<OpPayloadAttributes>,
}

#[derive(Clone, Debug)]
pub struct Sequencer<Db, DbRead> {
    state: SequencerState<DbRead>,
    data: SharedData<Db, DbRead>,
}

impl<Db: BopDB, DbRead: BopDbRead> Sequencer<Db, DbRead> {
    pub fn new(db: Db, frag_db: DBFrag<DbRead>, runtime: Arc<Runtime>, config: SequencerConfig) -> Self {
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

impl<Db: BopDB, DbRead: BopDbRead> Actor<DbRead> for Sequencer<Db, DbRead> {
    const CORE_AFFINITY: Option<usize> = Some(0);

    fn loop_body(&mut self, connections: &mut Connections<SendersSpine<DBFrag<DbRead>>, ReceiversSpine<DBFrag<DbRead>>>) {
        connections.receive(|msg: SimulatorToSequencer, _| {
            todo!();
        });

        connections.receive(|msg, senders| {
            self.handle_engine_api_message(msg, senders);
        });

        connections.receive(|msg: Arc<Transaction>, senders| {
            info!("received msg from ethapi");
            todo!();
            // self.tx_pool.handle_new_tx(msg, &self.db, DEFAULT_BASE_FEE, Some(senders));
        });

        connections.receive(|msg, _| {
            // Process blocks as they arrive
            self.handle_block(msg);
        });
    }
}

impl<Db: BopDB, DbRead: BopDbRead> Sequencer<Db, DbRead> {
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
    fn handle_engine_api_message(&self, msg: messages::EngineApi, senders: &SendersSpine<DBFrag<DbRead>>) {
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
