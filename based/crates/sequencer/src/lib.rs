use std::sync::Arc;

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
}

impl<Db: DatabaseRef> Sequencer<Db> {
    pub fn new(db: Db, runtime: Arc<Runtime>) -> Self {
        Self { db, tx_pool: TxPool::default(), runtime }
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
                payload: _,
                versioned_hashes: _,
                parent_beacon_block_root: _,
                res_tx: _,
            } => {
                // TODO: apply new payload
            }
            messages::EngineApi::ForkChoiceUpdatedV3 { fork_choice_state: _, payload_attributes: _, res_tx: _ } => {}
            messages::EngineApi::GetPayloadV3 { payload_id: _, res: _ } => {}
        }
    }
}
