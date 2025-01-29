use std::sync::Arc;

use bop_common::{
    actor::Actor,
    communication::{
        messages::SimulatorToSequencer,
        sequencer::{ReceiversSequencer, SendersSequencer},
        Connections, Spine,
    },
    transaction::Transaction,
};
use bop_pool::transaction::pool::TxPool;
use revm_primitives::db::DatabaseRef;
use tracing::info;

pub(crate) mod block_sync;

#[allow(dead_code)]
pub struct Sequencer<Db: DatabaseRef> {
    tx_pool: TxPool,
    db: Db,
}

impl<Db: DatabaseRef> Sequencer<Db> {
    pub fn new(db: Db) -> Self {
        Self { db, tx_pool: TxPool::default() }
    }
}

const DEFAULT_BASE_FEE: u64 = 10;

impl<Db> Actor for Sequencer<Db>
where
    Db: DatabaseRef + Send,
    <Db as DatabaseRef>::Error: std::fmt::Debug,
{
    type Receivers = ReceiversSequencer;
    type Senders = SendersSequencer;

    const CORE_AFFINITY: Option<usize> = Some(0);

    fn loop_body(&mut self, connections: &mut Connections<SendersSequencer, ReceiversSequencer>) {
        connections.receive(|msg: SimulatorToSequencer, _| {
            info!("received {}", msg.as_ref());
            match msg {
                SimulatorToSequencer::SimulatedTxList(_) => {}
            };
        });

        connections.receive(|msg: Arc<Transaction>, senders| {
            info!("received msg from ethapi");
            self.tx_pool.handle_new_tx(msg, &self.db, DEFAULT_BASE_FEE, senders);
        });
    }

    fn create_senders(&self, spine: &Spine) -> Self::Senders {
        spine.into()
    }

    fn create_receivers(&self, spine: &Spine) -> Self::Receivers {
        Self::Receivers::new(self, spine)
    }
}
