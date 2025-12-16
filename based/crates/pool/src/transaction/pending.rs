use std::sync::Arc;

use alloy_consensus::Transaction as TxTrait;
use alloy_primitives::Address;
use bop_common::{
    order::bundle::SimulatedBundle,
    transaction::{SimulatedTxList, Transaction},
};
use rustc_hash::FxHashMap;
use uuid::Uuid;

/// Orders that are ready to be executed by the sequencer.
#[derive(Debug, Clone, Default)]
pub struct PendingOrders {
    /// Transactions, keyed by sender and ordered by nonce (ascending).
    transactions: FxHashMap<Address, SimulatedTxList>,
    /// Bundles, ordered by insertion order.
    bundles: FxHashMap<Uuid, SimulatedBundle>,
    /// Maps sender to indices in `bundles`.
    bundle_senders: FxHashMap<Address, Vec<Uuid>>,
    /// Next nonce for each sender.
    next_nonce: FxHashMap<Address, u64>,
}

impl PendingOrders {
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            transactions: FxHashMap::with_capacity_and_hasher(capacity, Default::default()),
            bundles: FxHashMap::with_capacity_and_hasher(capacity, Default::default()),
            bundle_senders: FxHashMap::with_capacity_and_hasher(capacity, Default::default()),
            next_nonce: FxHashMap::with_capacity_and_hasher(capacity, Default::default()),
        }
    }

    /// Adds a tx list to the pending orders, overriding any existing tx list with the same sender.
    pub fn put_tx_list(&mut self, list: SimulatedTxList) {
        let sender = list.sender();
        self.next_nonce.insert(sender, list.nonce());
        self.transactions.insert(sender, list);
    }

    /// Adds a bundle to the pending orders, overriding any existing bundle with the same id.
    pub fn put_bundle(&mut self, bundle: SimulatedBundle) {
        for tx in bundle.validated().transactions.iter() {
            self.next_nonce.insert(tx.sender(), tx.nonce());
        }

        self.bundles.insert(bundle.validated().id(), bundle);
    }

    #[inline]
    pub fn clear(&mut self) {
        self.transactions.clear();
        self.bundles.clear();
    }

    #[inline]
    pub fn next_nonce(&self, sender: Address) -> Option<u64> {
        self.next_nonce.get(&sender).cloned()
    }

    #[inline]
    pub fn tx_list(&self, sender: &Address) -> Option<&SimulatedTxList> {
        self.transactions.get(sender)
    }

    #[inline]
    pub fn tx_list_mut(&mut self, sender: &Address) -> Option<&mut SimulatedTxList> {
        self.transactions.get_mut(sender)
    }

    /// Removes all transactions with nonce lower or equal than the provided threshold.
    #[inline]
    pub fn forward(&mut self, sender: &Address, nonce: u64, f: &mut impl FnMut(Arc<Transaction>)) {
        let Some(list) = self.tx_list_mut(sender) else {
            return;
        };

        if list.pending.forward(nonce, f) {
            self.transactions.remove(sender);
            // We can return early here because it's not possible for any other bundles to be invalidated by this nonce.
            return;
        }

        if let Some(ref current) = list.current {
            if nonce >= current.nonce() {
                list.current = None;
            }
        }

        // Get all indices for that sender
        // Check the bundles, remove the ones that are invalidated by (sender, nonce)
        // If changed, update the indices for the sender.
        let mut to_remove = Vec::new();
        if let Some(indices) = self.bundle_senders.get(sender) {
            for id in indices {
                // If there's any transaction in this bundle that gets invalidated by the forward, remove the bundle.
                if self.bundles.get(id).unwrap().validated().transactions.iter().any(|tx| tx.nonce() >= nonce) {
                    to_remove.push(*id);
                }
            }
        }

        for id in to_remove {
            self.bundles.remove(&id);
            self.bundle_senders.get_mut(sender).unwrap().retain(|id2| id2 != &id);
        }
    }

    /// Returns a snapshot of the pending orders (clones internally).
    pub fn view(&self) -> PendingOrdersView {
        PendingOrdersView { transactions: self.transactions.clone(), bundles: self.bundles.clone() }
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.transactions.is_empty() && self.bundles.is_empty()
    }
}

/// A snapshot of the pending orders.
pub struct PendingOrdersView {
    pub transactions: FxHashMap<Address, SimulatedTxList>,
    pub bundles: FxHashMap<Uuid, SimulatedBundle>,
}
