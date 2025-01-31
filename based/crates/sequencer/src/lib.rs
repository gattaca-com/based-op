use std::sync::Arc;

use alloy_consensus::Block;
use bop_common::{
    actor::Actor,
    communication::{
        messages::{self, SimulatorToSequencer},
        Connections, ReceiversSpine, SendersSpine,
    },
    transaction::Transaction,
};
use bop_db::BopDbRead;
use bop_pool::transaction::pool::TxPool;
use reth_optimism_primitives::OpTransactionSigned;
use reth_primitives::BlockWithSenders;
use revm_primitives::db::DatabaseRef;
use tokio::runtime::Runtime;
use tracing::info;

use crate::block_sync::fetch_blocks::fetch_blocks_and_send_sequentially;

pub(crate) mod block_sync;

#[allow(dead_code)]
pub struct Sequencer<Db> {
    tx_pool: TxPool,
    db: Db,
    runtime: Arc<Runtime>,

    /// Used for fetching blocks from the RPC when our db is behind the chain head.
    /// Blocks are fetched async and returned to the sequencer through this channel.
    sender_fetch_blocks_to_sequencer:
        crossbeam_channel::Sender<Result<BlockWithSenders<Block<OpTransactionSigned>>, reqwest::Error>>,
    receiver_fetch_blocks_to_sequencer:
        crossbeam_channel::Receiver<Result<BlockWithSenders<Block<OpTransactionSigned>>, reqwest::Error>>,
}

impl<Db: DatabaseRef> Sequencer<Db> {
    pub fn new(db: Db, runtime: Arc<Runtime>) -> Self {
        let (sender_fetch_blocks_to_sequencer, receiver_fetch_blocks_to_sequencer) = crossbeam_channel::bounded(200);
        Self {
            db,
            tx_pool: TxPool::default(),
            runtime,
            sender_fetch_blocks_to_sequencer,
            receiver_fetch_blocks_to_sequencer,
        }
    }
}

const DEFAULT_BASE_FEE: u64 = 10;

impl<Db> Actor<Db> for Sequencer<Db>
where
    Db: BopDbRead + Send,
    <Db as DatabaseRef>::Error: std::fmt::Debug,
{
    const CORE_AFFINITY: Option<usize> = Some(0);

    fn loop_body(&mut self, connections: &mut Connections<SendersSpine<Db>, ReceiversSpine<Db>>) {
        connections.receive(|msg: SimulatorToSequencer, _| {
            info!("received {}", msg.as_ref());
            match msg {
                SimulatorToSequencer::SimulatedTxList(_) => {}
            };
        });

        connections.receive(|msg: messages::EngineApi, _| {
            info!("received msg from engine api");
            self.handle_engine_api_message(msg);
        });

        connections.receive(|msg: Arc<Transaction>, senders| {
            info!("received msg from ethapi");
            self.tx_pool.handle_new_tx(msg, &self.db, DEFAULT_BASE_FEE, senders);
        });
    }
}

impl<Db: DatabaseRef + BopDbRead> Sequencer<Db> {
    /// Handles messages from the engine API.
    ///
    /// - `NewPayloadV3` triggers a block sync if the payload is for a new block.
    fn handle_engine_api_message(&self, msg: messages::EngineApi) {
        match msg {
            messages::EngineApi::NewPayloadV3 {
                payload,
                versioned_hashes: _,
                parent_beacon_block_root: _,
                res_tx: _,
            } => {
                let seq_block_number = self.db.block_number().expect("failed to get block number from db");
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
                        self.sender_fetch_blocks_to_sequencer.clone(),
                        &self.runtime,
                    );

                    // Process blocks as they arrive
                    while let Ok(block_result) = self.receiver_fetch_blocks_to_sequencer.try_recv() {
                        let block = block_result.expect("failed to fetch block");
                        // TODO: apply block

                        if block.header.number == payload_block_number - 1 {
                            break;
                        }
                    }
                }

                // TODO: apply new payload
            }
            messages::EngineApi::ForkChoiceUpdatedV3 { fork_choice_state: _, payload_attributes: _, res_tx: _ } => {}
            messages::EngineApi::GetPayloadV3 { payload_id: _, res: _ } => {}
        }
    }
}
