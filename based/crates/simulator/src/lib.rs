use std::sync::Arc;

use bop_common::{
    actor::Actor,
    communication::{
        messages::SequencerToSimulator,
        simulator::{ReceiversSimulator, SendersSimulator},
        Connections, Spine, TrackedSenders,
    },
    time::Duration,
    transaction::Transaction,
    utils::last_part_of_typename,
};
use bop_db::BopDB;
use tracing::info;

pub struct Simulator<Db: BopDB> {
    id: usize,
    _db: Db,
}

impl<Db: BopDB> Simulator<Db> {
    pub fn new(db: Db, id: usize) -> Self {
        Self { id, _db: db }
    }

    fn simulate_tx_list(&self, tx_list: Vec<Arc<Transaction>>) -> Vec<Arc<Transaction>> {
        tx_list
    }
}

impl<Db: BopDB> Actor for Simulator<Db> {
    type Receivers = ReceiversSimulator;
    type Senders = SendersSimulator;

    const CORE_AFFINITY: Option<usize> = None;

    fn name(&self) -> String {
        format!("{}-{}", last_part_of_typename::<Self>(), self.id)
    }

    fn loop_body(&mut self, connections: &mut Connections<SendersSimulator, ReceiversSimulator>) {
        connections.receive(|msg, senders| {
            info!("received {}", msg.as_ref());
            match msg {
                SequencerToSimulator::SimulateTxList(txs) => {
                    debug_assert!(
                        senders
                            .send_timeout(
                                bop_common::communication::messages::SimulatorToSequencer::SimulatedTxList(
                                    self.simulate_tx_list(txs)
                                ),
                                Duration::from_millis(10)
                            )
                            .is_ok(),
                        "timed out trying to send request"
                    );
                }
                SequencerToSimulator::NewBlock => {
                    todo!()
                }
            }
        });
    }

    fn create_senders(&self, spine: &Spine) -> Self::Senders {
        spine.into()
    }

    fn create_receivers(&self, spine: &Spine) -> Self::Receivers {
        Self::Receivers::new(self, spine)
    }
}
