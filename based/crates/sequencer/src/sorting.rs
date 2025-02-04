use std::{collections::VecDeque, ops::Deref, sync::Arc};

use bop_common::{
    communication::{
        messages::{SequencerToSimulator, SimulationResult},
        SpineConnections,
    },
    db::BopDbRead,
    time::Instant,
    transaction::{SimulatedTx, SimulatedTxList, Transaction},
};
use revm_primitives::{Address, B256};
use tracing::error;

use crate::frag::InSortFrag;

#[derive(Clone, Debug, Default)]
pub struct ActiveOrders {
    orders: Vec<SimulatedTxList>,
}

impl ActiveOrders {
    pub fn new(orders: Vec<SimulatedTxList>) -> Self {
        Self { orders }
    }

    fn len(&self) -> usize {
        self.orders.len()
    }

    pub fn remove_from_sender(&mut self, sender: &Address, base_fee: u64) {
        for i in (0..self.len() - 1).rev() {
            let order = &mut self.orders[i];
            if &order.sender() == sender && order.pop(base_fee) {
                self.orders.swap_remove(i);
                return;
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

impl Deref for ActiveOrders {
    type Target = Vec<SimulatedTxList>;

    fn deref(&self) -> &Self::Target {
        &self.orders
    }
}

/// State of the sequencer while sorting frags
#[derive(Clone, Debug)]
pub struct SortingData<Db: BopDbRead> {
    /// Current frag being sorted
    pub frag: InSortFrag<Db>,
    /// Deadline when to seal the current frag
    pub until: Instant,
    /// How many simulations we are waiting for
    pub in_flight_sims: usize,
    /// All orders simulated on top of the current frag
    pub tof_snapshot: ActiveOrders,
    /// Next best order to apply
    pub next_to_be_applied: Option<SimulatedTx>,
    /// Txs in payload attributes that need to be applied in order
    pub remaining_attributes_txs: VecDeque<Arc<Transaction>>,
    /// Whether we can add transactions other than the ones in the attributes
    pub can_add_txs: bool,
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

        if let Some(tx) = self.remaining_attributes_txs.pop_front() {
            senders.send(SequencerToSimulator::SimulateTx(tx, db.clone()));
            self.in_flight_sims += 1;
        } else if self.can_add_txs {
            for t in self.tof_snapshot.iter().rev().take(n_sims_per_loop).map(|t| t.next_to_sim()) {
                debug_assert!(t.is_some(), "Unsimmable TxList should have been cleared previously");
                let tx = t.unwrap();
                senders.send(SequencerToSimulator::SimulateTx(tx, db.clone()));
                self.in_flight_sims += 1;
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

    pub fn should_seal_frag(&self) -> bool {
        self.until < Instant::now()
    }

    pub fn should_send_next_sims(&self) -> bool {
        self.in_flight_sims == 0
    }
}
