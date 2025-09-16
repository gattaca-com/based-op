use std::{
    collections::VecDeque,
    ops::{Deref, DerefMut},
};

use bop_common::transaction::{SimulatedTx, SimulatedTxList};
use revm_primitives::{Address, U256};
use tracing::debug;

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

    pub fn available_value(&self) -> U256 {
        self.orders.iter().map(|t| t.current.as_ref().map(|tx| tx.payment).unwrap_or_default()).sum()
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
                    self.orders.swap_remove_back(i).unwrap();
                }
                return;
            }
        }
        unreachable!("this should never happen");
    }

    pub fn put(&mut self, tx: SimulatedTx, fifo_ordering: bool) {
        let mut id = self.orders.len();

        if !fifo_ordering {
            let payment = tx.payment;
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
        }
        // not found so we insert it at the id corresponding to the payment
        self.orders.insert(id, SimulatedTxList::from(tx))
    }

    /// Checks whether we have enough gas remaining for order at id.
    pub fn not_enough_gas(&mut self, id: usize, gas_remaining: u64) -> bool {
        self.orders[id].gas_limit().is_none_or(|gas| gas_remaining < gas)
    }

    /// Checks whether the DA requirement for the order at `id` exceeds the available DA.
    pub fn da_too_big(&mut self, id: usize, da_remaining: Option<u64>, max_da: Option<u64>) -> bool {
        (match max_da {
            Some(remaining) => {
                let da = self.orders[id].estimated_da();
                let too_large = da.is_none_or(|da| da > remaining);
                if too_large {
                    debug!(?id, ?max_da, ?da, "tx DA too large");
                }
                too_large
            }
            None => false,
        }) || (match da_remaining {
            Some(remaining) => {
                let da = self.orders[id].estimated_da();
                let too_large = da.is_none_or(|da| da > remaining);
                if too_large {
                    debug!(?id, ?da_remaining, ?da, "tx DA too large for block");
                }
                too_large
            }
            None => false,
        })
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
