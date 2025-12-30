use std::{
    collections::VecDeque,
    ops::{Deref, DerefMut},
};

use bop_common::order::{PendingOrder, SimulatedOrder};
use revm_primitives::{Address, U256};
use tracing::debug;

pub(crate) mod sorting_data;
pub(crate) use sorting_data::SortingData;
pub(crate) mod frag_sequence;
pub(crate) use frag_sequence::FragSequence;

#[derive(Clone, Debug, Default)]
pub struct ActiveOrders {
    orders: VecDeque<PendingOrder>,
}

impl ActiveOrders {
    pub fn new(mut orders: Vec<PendingOrder>, fifo_ordering: bool) -> Self {
        if fifo_ordering {
            // NOTE: This function is used to populate the `tof_snaphost`, where a new transaction
            // is pushed front on a `VecDeque`. Instead, a new active transaction in the tx pool
            // is pushed back on a `Vec`, so since we need to maintain ordering here we have to
            // reverse the list. That is, most recent transactions first.
            orders.reverse();
        } else {
            // WARNING: this might lead to apples to oranges comparison if we haven't
            // re-simulated all forwarded txlists top of last applied frag in the pool Activelist.
            // This is currently the situation
            orders.sort_unstable_by_key(|t| t.weight());
        }
        Self { orders: orders.into() }
    }

    pub fn empty() -> Self {
        Self { orders: Default::default() }
    }

    fn len(&self) -> usize {
        self.orders.len()
    }

    /// Returns the total available value of the orders, i.e. the sum of the simulated payments of the orders.
    pub fn available_value(&self) -> U256 {
        self.orders.iter().map(|t| t.payment().unwrap_or_default()).sum()
    }

    /// Removes all pending txs for a sender list.
    /// We remove all as nonces needed to be mined in sequential order.
    pub fn remove_from_sender(&mut self, sender: Address, base_fee: u64) {
        if self.is_empty() {
            return;
        }

        let len = self.orders.len();
        let mut to_remove = Vec::new();

        for (i, order) in self.orders.iter_mut().rev().enumerate() {
            // Get the actual index of the order in the deque.
            let index = len - i - 1;

            match order {
                PendingOrder::Tx(list) => {
                    if list.sender() == sender && list.pop(base_fee) {
                        to_remove.push(index);
                    }
                }
                PendingOrder::Bundle(bundle) => {
                    if bundle.has_sender(sender) {
                        // TODO: Needs any additional checks?
                        to_remove.push(index);
                    }
                }
            }
        }

        for index in to_remove {
            self.orders.swap_remove_back(index).unwrap();
        }
    }

    pub fn put(&mut self, order: SimulatedOrder, fifo_ordering: bool) {
        let mut id = self.orders.len();

        if !fifo_ordering {
            let payment = order.payment();

            match order {
                SimulatedOrder::Tx(ref tx) => {
                    let sender = tx.sender();
                    for (i, order) in self.orders.iter_mut().enumerate().rev() {
                        let Some(order) = order.as_tx_list_mut() else {
                            return;
                        };

                        if order.sender() == sender {
                            order.put(tx.clone());
                            return;
                        }

                        if payment < order.payment() {
                            id = i;
                        }
                    }
                }
                SimulatedOrder::Bundle(_) => {
                    for (i, order) in self.orders.iter_mut().enumerate().rev() {
                        let Some(order) = order.as_bundle() else {
                            return;
                        };

                        if payment < order.payment() {
                            id = i;
                        }
                    }
                }
            }
        }

        // not found so we insert it at the id corresponding to the payment
        self.orders.insert(id, PendingOrder::from(order))
    }

    /// Checks whether we have enough gas remaining for order at id.
    pub fn not_enough_gas(&mut self, id: usize, gas_remaining: u64) -> bool {
        self.orders[id].gas_limit().is_none_or(|gas| gas_remaining < gas)
    }

    /// Checks whether the DA requirement for the order at `id` exceeds the available DA.
    pub fn da_too_big(
        &mut self,
        id: usize,
        block_da_remaining: Option<u64>,
        tx_max_da: Option<u64>,
        block_gas_limit: u64,
        da_footprint_gas_scalar: Option<u64>,
        da_used: u64,
    ) -> bool {
        if let Some(tx_max_da) = tx_max_da {
            let da = self.orders[id].estimated_da();
            let too_large = da.is_none_or(|da| da > tx_max_da);
            if too_large {
                debug!(?id, ?tx_max_da, ?da, "tx DA too large");
                return true;
            }
        }

        if let Some(block_da_remaining) = block_da_remaining {
            let da = self.orders[id].estimated_da();
            let too_large = da.is_none_or(|da| da > block_da_remaining);
            if too_large {
                debug!(?id, ?block_da_remaining, ?da, "tx DA too large for block");
                return true;
            }
        }

        // Post Jovian: the tx DA footprint must be less than the block gas limit
        if let Some(da_footprint_gas_scalar) = da_footprint_gas_scalar {
            let tx_da_size = self.orders[id].estimated_da();
            if let Some(tx_da_size) = tx_da_size {
                // Calculate total DA bytes used if we add this transaction
                let total_da_bytes_used = da_used + tx_da_size;

                // Calculate DA footprint in gas: total_da_bytes_used * da_footprint_gas_scalar
                let tx_da_footprint = total_da_bytes_used.saturating_mul(da_footprint_gas_scalar);

                // Check if adding this transaction would exceed the block gas limit
                if tx_da_footprint > block_gas_limit {
                    debug!(
                        ?id,
                        ?total_da_bytes_used,
                        ?tx_da_size,
                        ?tx_da_footprint,
                        ?block_gas_limit,
                        "tx DA footprint exceeds block gas limit"
                    );
                    return true;
                }
            }
        }

        false
    }
}

impl Deref for ActiveOrders {
    type Target = VecDeque<PendingOrder>;

    fn deref(&self) -> &Self::Target {
        &self.orders
    }
}
impl DerefMut for ActiveOrders {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.orders
    }
}
