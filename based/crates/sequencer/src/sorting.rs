use std::ops::Deref;

use bop_common::{
    communication::{
        messages::{SequencerToSimulator, SimulationResult},
        ReceiversSpine, SendersSpine, SpineConnections, TrackedSenders,
    },
    db::{BopDB, BopDbRead},
    p2p::FragMessage,
    time::{Duration, Instant},
    transaction::{SimulatedTx, SimulatedTxList},
};
use revm_primitives::{Address, B256};
use tracing::error;

use crate::{built_frag::BuiltFrag, SharedData};

#[derive(Clone, Debug, Default)]
pub struct BuiltFragOrders {
    orders: Vec<SimulatedTxList>,
}

impl BuiltFragOrders {
    fn len(&self) -> usize {
        self.orders.len()
    }

    pub fn remove_from_sender(&mut self, sender: &Address, base_fee: u64) {
        for i in (0..self.len() - 1).rev() {
            let order = &mut self.orders[i];
            if &order.sender() == sender {
                if order.pop(base_fee) {
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
                order.put(tx);
                return;
            }
        }
    }
}

impl Deref for BuiltFragOrders {
    type Target = Vec<SimulatedTxList>;

    fn deref(&self) -> &Self::Target {
        &self.orders
    }
}

impl<Db: BopDB> From<&SharedData<Db>> for BuiltFragOrders {
    fn from(value: &SharedData<Db>) -> Self {
        Self { orders: value.tx_pool.clone_active() }
    }
}

#[derive(Clone, Debug)]
pub struct SortingData<Db: BopDbRead> {
    /// This is the db that is built on top of the last block chunk to be used to
    /// build a new cachedb on top of for sorting
    /// starting a new sort
    frag: BuiltFrag<Db>,
    until: Instant,
    in_flight_sims: usize,
    tof_snapshot: BuiltFragOrders,
    next_to_be_applied: Option<SimulatedTx>,
}
impl<Db: BopDbRead> SortingData<Db> {
    pub fn apply_and_send_next(
        mut self,
        n_sims_per_loop: usize,
        senders: &mut SpineConnections<Db>,
        base_fee: u64,
    ) -> Self {
        if let Some(tx_to_apply) = std::mem::take(&mut self.next_to_be_applied) {
            self.tof_snapshot.remove_from_sender(&tx_to_apply.sender(), base_fee);
            self.frag.apply_tx(tx_to_apply);
        }

        let db = self.frag.state();

        for t in self.tof_snapshot.iter().rev().take(n_sims_per_loop).map(|t| t.next_to_sim()) {
            debug_assert!(t.is_some(), "Unsimmable TxList should have been cleared previously");
            let tx = t.unwrap();
            if senders.send(SequencerToSimulator::SimulateTx(tx, db.clone())).is_ok() {
                self.in_flight_sims += 1
            }
        }
        self
    }

    pub fn is_valid(&self, unique_hash: B256) -> bool {
        unique_hash != self.frag.db.unique_hash()
    }

    pub fn handle_sim(&mut self, simulated_tx: SimulationResult<SimulatedTx, Db>, sender: Address, base_fee: u64) {
        self.in_flight_sims -= 1;

        // handle errored sim
        let Ok(simulated_tx) = simulated_tx.inspect_err(|e| error!("simming tx for sender {sender} {e}",)) else {
            self.tof_snapshot.remove_from_sender(&sender, base_fee);
            return;
        };

        let tx_to_put_back = if self.next_to_be_applied.as_ref().is_none_or(|t| t.payment < simulated_tx.payment) {
            self.next_to_be_applied.replace(simulated_tx)
        } else {
            Some(simulated_tx)
        };
        if let Some(tx) = tx_to_put_back {
            self.tof_snapshot.put(tx)
        }
    }

    pub(crate) fn should_frag(&self) -> bool {
        self.until < Instant::now()
    }

    pub(crate) fn finished_iteration(&self) -> bool {
        self.in_flight_sims == 0
    }

    pub(crate) fn get_frag(&self) -> FragMessage {
        todo!()
    }
}

impl<Db: BopDB> From<&SharedData<Db>> for SortingData<Db::ReadOnly> {
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
