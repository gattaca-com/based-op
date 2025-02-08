use std::ops::Deref;

use bop_common::transaction::{SimulatedTx, SimulatedTxList};
use revm_primitives::Address;

pub(crate) mod sorting_data;
pub(crate) use sorting_data::SortingData;
pub(crate) mod frag_sequence;

pub(crate) use frag_sequence::FragSequence;

#[derive(Clone, Debug, Default)]
pub struct ActiveOrders {
    orders: Vec<SimulatedTxList>,
}

impl ActiveOrders {
    pub fn new(mut orders: Vec<SimulatedTxList>) -> Self {
        // WARNING: this might lead to apples to oranges comparison if we haven't
        // re-simulated all forwarded txlists top of last applied frag in the pool Activelist.
        // This is currently the situation
        orders.sort_unstable_by_key(|t| t.weight());
        Self { orders }
    }

    pub fn empty() -> Self {
        Self { orders: vec![] }
    }

    fn len(&self) -> usize {
        self.orders.len()
    }

    /// Removes all pending txs for a sender list.
    /// We remove all as nonces needed to be mined in sequential order.
    pub fn remove_from_sender(&mut self, sender: &Address, base_fee: u64) {
        if self.is_empty() {
            return;
        }
        for i in (0..self.len()).rev() {
            let order = &mut self.orders[i];
            // tracing::info!("checked from {} vs {sender}", order.sender());
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
