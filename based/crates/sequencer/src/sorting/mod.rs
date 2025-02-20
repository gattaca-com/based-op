use std::{
    collections::VecDeque,
    ops::{Deref, DerefMut},
};

use bop_common::transaction::{SimulatedTx, SimulatedTxList};
use revm_primitives::Address;

pub(crate) mod sorting_data;
pub(crate) use sorting_data::SortingData;
pub(crate) mod frag_sequence;

pub(crate) use frag_sequence::FragSequence;

#[derive(Clone, Debug, Default)]
pub struct ActiveOrders {
    orders: VecDeque<SimulatedTxList>,
}

impl ActiveOrders {
    pub fn new(mut orders: Vec<SimulatedTxList>) -> Self {
        // WARNING: this might lead to apples to oranges comparison if we haven't
        // re-simulated all forwarded txlists top of last applied frag in the pool Activelist.
        // This is currently the situation
        orders.sort_unstable_by_key(|t| t.weight());
        Self { orders: orders.into() }
    }

    pub fn empty() -> Self {
        Self { orders: Default::default() }
    }

    fn len(&self) -> usize {
        self.orders.len()
    }

    /// Removes all pending txs for a sender list.
    /// We remove all as nonces needed to be mined in sequential order.
    pub fn remove_from_sender(&mut self, sender: Address, base_fee: u64) {
        if self.is_empty() {
            return;
        }
        for i in (0..self.len()).rev() {
            let order = &mut self.orders[i];
            debug_assert_ne!(order.sender(), Address::default(), "should never have an order with default sender");

            if order.sender() == sender {
                if order.pop(base_fee) {
                    self.orders.swap_remove_back(i);
                }
                return;
            }
        }
        unreachable!("this should never happen");
    }

    pub fn put(&mut self, tx: SimulatedTx) {
        let payment = tx.payment;
        let mut id = self.orders.len();
        let sender = tx.sender();
        for (i, order) in self.orders.iter_mut().enumerate().rev() {
            if order.sender() == sender {
                order.put(tx);
                return;
            }
            if payment < order.payment() {
                id = i;
            }
        }
        // not found so we insert it at the id corresponding to the payment
        self.orders.insert(id, SimulatedTxList::from(tx))
    }

    /// Checks whether we have enough gas remaining for order at id.
    pub fn not_enough_gas(&mut self, id: usize, gas_remaining: u64) -> bool {
        self.orders[id].gas_limit().is_none_or(|gas| gas_remaining < gas)
    }
}

impl Deref for ActiveOrders {
    type Target = VecDeque<SimulatedTxList>;

    fn deref(&self) -> &Self::Target {
        &self.orders
    }
}
impl DerefMut for ActiveOrders {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.orders
    }
}
