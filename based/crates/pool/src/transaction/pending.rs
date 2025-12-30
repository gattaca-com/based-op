use std::sync::Arc;

use alloy_consensus::Transaction as TxTrait;
use alloy_primitives::Address;
use bop_common::{
    order::{PendingOrder, bundle::SimulatedBundle},
    transaction::{SimulatedTxList, Transaction},
};
use indexmap::IndexMap;
use rustc_hash::FxHashMap;
use uuid::Uuid;

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
enum OrderKey {
    Tx(Address),
    Bundle(Uuid),
}

impl From<Address> for OrderKey {
    fn from(sender: Address) -> Self {
        OrderKey::Tx(sender)
    }
}

impl From<Uuid> for OrderKey {
    fn from(id: Uuid) -> Self {
        OrderKey::Bundle(id)
    }
}

/// State for a sender present in the pending orders.
#[derive(Debug, Clone)]
struct SenderState {
    /// The current nonce for this sender.
    nonce: u64,
    /// The entries for this sender in the main order map.
    entries: Vec<OrderKey>,
}

/// Pending orders that are ready to be executed by the sequencer. All nonces are correct, i.e. there are no gaps or
/// duplicate (sender, nonce) pairs.
#[derive(Debug, Clone, Default)]
pub struct PendingOrders {
    /// All senders with their state and link to the main order map.
    senders: FxHashMap<Address, SenderState>,
    /// All orders by insertion order.
    orders: IndexMap<OrderKey, PendingOrder>,
}

impl PendingOrders {
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            senders: FxHashMap::with_capacity_and_hasher(capacity, Default::default()),
            orders: IndexMap::with_capacity_and_hasher(capacity, Default::default()),
        }
    }

    /// Adds a tx list to the pending orders, overriding any existing tx list with the same sender.
    pub fn put_tx_list(&mut self, list: SimulatedTxList) {
        let sender = list.sender();
        let key = OrderKey::from(sender);

        let entry =
            self.senders.entry(sender).or_insert_with(|| SenderState { nonce: list.nonce(), entries: Vec::new() });

        entry.entries.push(key.clone());
        entry.nonce = list.nonce();

        self.orders.insert(key, PendingOrder::Tx(list));
    }

    /// Adds a bundle to the pending orders, overriding any existing bundle with the same id.
    pub fn put_bundle(&mut self, bundle: SimulatedBundle) {
        let key = OrderKey::from(bundle.validated_ref().id());
        for tx in bundle.validated_ref().transactions.iter() {
            let entry = self
                .senders
                .entry(tx.sender())
                .or_insert_with(|| SenderState { nonce: tx.nonce(), entries: Vec::new() });

            entry.entries.push(key.clone());

            if tx.nonce() > entry.nonce {
                entry.nonce = tx.nonce();
            }
        }

        self.orders.insert(key, PendingOrder::Bundle(bundle));
    }

    #[inline]
    pub fn clear(&mut self) {
        self.senders.clear();
        self.orders.clear();
    }

    #[inline]
    pub fn next_nonce(&self, sender: Address) -> Option<u64> {
        self.senders.get(&sender).map(|state| state.nonce)
    }

    #[inline]
    pub fn tx_list(&self, sender: &Address) -> Option<&SimulatedTxList> {
        self.orders.get(&OrderKey::from(*sender)).and_then(|order| order.as_tx_list())
    }

    #[inline]
    pub fn tx_list_mut(&mut self, sender: &Address) -> Option<&mut SimulatedTxList> {
        self.orders.get_mut(&OrderKey::from(*sender)).and_then(|order| order.as_tx_list_mut())
    }

    /// Removes all transactions with nonce lower or equal than the provided threshold.
    #[inline]
    pub fn forward(&mut self, sender: &Address, nonce: u64, f: &mut impl FnMut(Arc<Transaction>)) {
        let Some(state) = self.senders.get_mut(sender) else {
            return;
        };

        // Only proceed if we have a nonce that is <= this nonce.
        if state.nonce > nonce {
            return;
        }

        let mut to_remove = Vec::new();
        for (index, entry) in state.entries.iter().enumerate() {
            let Some(order) = self.orders.get_mut(entry) else {
                continue;
            };

            match order {
                PendingOrder::Tx(list) => {
                    if list.pending.forward(nonce, f) {
                        self.orders.shift_remove(entry);
                        to_remove.push(index);

                        continue;
                    }

                    if let Some(ref current) = list.current {
                        if nonce >= current.nonce() {
                            list.current = None;
                        }
                    }
                }

                PendingOrder::Bundle(bundle) => {
                    if bundle.validated_ref().transactions.iter().any(|tx| tx.nonce() <= nonce) {
                        self.orders.shift_remove(entry);
                        to_remove.push(index);
                    }
                }
            }
        }

        // Remove the stale entries from the sender state.
        // NOTE: For bundles, we should technically also remove the pointers from other senders for any invalidated
        // bundles. We omit this for now because the state is cleared every frag.
        for index in to_remove.iter().rev() {
            state.entries.swap_remove(*index);
        }

        // Set the nonce to the next nonce for this sender, since everything below it has been removed.
        state.nonce = nonce.saturating_add(1);

        if state.entries.is_empty() {
            self.senders.remove(sender);
        }
    }

    /// Returns a snapshot of the pending orders (clones internally) in insertion order.
    pub fn snapshot(&self) -> impl Iterator<Item = PendingOrder> + '_ {
        self.orders.values().cloned()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.orders.is_empty()
    }
}
