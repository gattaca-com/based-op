use std::{ops::Deref, slice::Iter, sync::Arc};

use alloy_consensus::Transaction as AlloyTransactionTrait;

use crate::transaction::Transaction;

/// A nonce-sorted list of transactions from a single sender.
#[derive(Clone, Debug, Default)]
pub struct TxList {
    txs: Vec<Arc<Transaction>>,
}

impl TxList {
    /// Inserts a new transaction into the ordered list, maintaining nonce order.
    /// If a transaction already exists with the same nonce, it's overwritten.
    #[inline]
    pub fn put(&mut self, new_tx: Arc<Transaction>) {
        let new_nonce = new_tx.nonce();

        if self.txs.is_empty() || self.txs[self.txs.len() - 1].nonce() < new_nonce {
            self.txs.push(new_tx);
            return;
        }

        match self.txs.binary_search_by_key(&new_nonce, |tx| tx.nonce()) {
            Ok(index) => self.txs[index] = new_tx,
            Err(index) => self.txs.insert(index, new_tx),
        }
    }

    /// Removes all transactions with nonce lower than the provided threshold.
    /// Returns true if list becomes empty after removal.
    #[inline]
    pub fn forward(&mut self, nonce: &u64) -> bool {
        let len = self.txs.len();
        if len == 0 {
            return true;
        }

        if self.txs[0].nonce() >= *nonce {
            return false;
        }

        // if last tx nonce is < target, clear all
        if self.txs[len - 1].nonce() < *nonce {
            self.txs.drain(..);
            return true;
        }

        // Find split point
        let split_idx = match self.txs.binary_search_by_key(nonce, |tx| tx.nonce()) {
            Ok(idx) => idx + 1,
            Err(idx) => idx,
        };

        if split_idx > 0 {
            self.txs.drain(0..split_idx);
        }

        self.txs.is_empty()
    }

    /// Ready retrieves a sequentially increasing list of transactions starting at the
    /// provided nonce that is ready for processing. Only txs with gas_price > base_fee
    /// are included.
    #[inline]
    pub fn ready(&self, curr_nonce: &mut u64, base_fee: u64) -> Option<Vec<Arc<Transaction>>> {
        if self.is_empty() || self.peek_nonce().unwrap() > *curr_nonce {
            return None;
        }

        let mut ready_txs = vec![];
        for next_tx in self.iter() {
            if next_tx.nonce() != *curr_nonce ||
                next_tx.gas_price_or_max_fee().map_or(false, |price| price < base_fee as u128)
            {
                break;
            }

            *curr_nonce += 1;
            ready_txs.push(next_tx.clone());
        }

        if ready_txs.is_empty() {
            return None;
        }

        Some(ready_txs)
    }

    /// Returns effective gas price at base_fee for tx with given nonce, or 0 if not found
    #[inline]
    pub fn get_effective_price_for_nonce(&self, nonce: &u64, base_fee: u64) -> u128 {
        self.get(nonce).map_or(0, |tx| tx.effective_gas_price(Some(base_fee)))
    }

    /// Retrieves a transaction with the given nonce from the transaction list.
    /// Returns a reference to the transaction if it exists.
    #[inline]
    pub fn get(&self, nonce: &u64) -> Option<&Arc<Transaction>> {
        self.txs.binary_search_by_key(nonce, |tx| tx.nonce()).ok().map(|index| &self.txs[index])
    }

    pub fn len(&self) -> usize {
        self.txs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.txs.is_empty()
    }

    pub fn iter(&self) -> Iter<'_, Arc<Transaction>> {
        self.txs.iter()
    }

    /// Returns the nonce of the first transaction in the list
    pub fn peek_nonce(&self) -> Option<u64> {
        self.txs.first().map(|tx| tx.tx.nonce())
    }

    /// Returns a reference to the first transaction in the list
    pub fn peek(&self) -> Option<&Arc<Transaction>> {
        self.txs.first()
    }

    pub fn contains(&self, nonce: &u64) -> bool {
        self.txs.binary_search_by_key(nonce, |tx| tx.nonce()).is_ok()
    }
}

impl From<Arc<Transaction>> for TxList {
    fn from(tx: Arc<Transaction>) -> Self {
        Self { txs: vec![tx] }
    }
}

impl From<Vec<Arc<Transaction>>> for TxList {
    fn from(txs: Vec<Arc<Transaction>>) -> Self {
        Self { txs }
    }
}

impl Deref for TxList {
    type Target = Arc<Transaction>;

    fn deref(&self) -> &Self::Target {
        &self.txs[0]
    }
}
