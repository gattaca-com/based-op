use std::sync::Arc;

use alloy_consensus::Transaction as AlloyTransactionTrait;
use revm_primitives::{Address, B256};

use crate::transaction::{simulated::transaction::SimulatedTx, Transaction, TxList};

/// Current contains the current active tx for this sender.
/// i.e., current.nonce = state[address].nonce.
/// Pending contains all other txs for this sender in nonce order.
#[derive(Clone, Debug)]
pub struct SimulatedTxList {
    pub current: Option<SimulatedTx>,
    pub pending: TxList,
}

impl SimulatedTxList {
    /// Takes a TxList containing all txs for a sender and the simulated tx of the first tx in pending
    /// and returns a SimulatedTxList.
    ///
    /// Will optionally trim the current tx from the pending list.
    pub fn new(current: SimulatedTx, pending: &TxList) -> SimulatedTxList {
        let mut pending = pending.clone();

        // Remove current from pending
        if pending.peek_nonce().is_some_and(|nonce| current.nonce() == nonce) {
            pending.pop_front();
        }

        debug_assert!(
            pending.peek_nonce().map_or(true, |nonce| current.nonce() == nonce + 1),
            "pending tx list nonce must be consecutive from current"
        );

        SimulatedTxList { current: Some(current), pending }
    }

    /// Updates the pending tx list.
    /// Will optionally trim the current tx from the pending list.
    #[inline]
    pub fn new_pending(&mut self, mut pending: TxList) {
        if let Some(current) = &self.current {
            if pending.peek_nonce().is_some_and(|nonce| current.nonce() == nonce) {
                pending.pop_front();
            }
        }
        self.pending = pending;
    }

    pub fn len(&self) -> usize {
        self.pending.len() + self.current.is_some() as usize
    }

    pub fn hash(&self) -> B256 {
        self.current.as_ref().map(|t| t.tx_hash()).unwrap_or_else(|| self.pending.tx_hash())
    }

    /// Removes the active transaction for the sender from the list.
    /// Returns true if all transactions for this sender have now been applied.
    pub fn pop(&mut self, base_fee: u64) -> bool {
        debug_assert!(self.current.is_some(), "Tried popping on a SimulatedTxList with current None: {self:#?}");
        let nonce = self.current.take().unwrap().nonce();
        self.current = None;

        self.pending.is_empty() || self.pending.first_ready(nonce + 1, base_fee).is_none()
    }

    pub fn put(&mut self, tx: SimulatedTx) {
        if self.pending.peek_nonce().is_some_and(|nonce| nonce == tx.nonce()) {
            self.pending.pop_front();
        }
        self.current = Some(tx);
    }

    pub fn next_to_sim(&self) -> Option<Arc<Transaction>> {
        self.current.as_ref().map(|t| t.tx.clone()).or_else(|| self.pending.peek().cloned())
    }

    pub fn sender(&self) -> Address {
        if let Some(tx) = &self.current {
            tx.tx.sender
        } else {
            self.pending.peek().map(|t| t.sender).unwrap_or_default()
        }
    }

    pub fn push(&mut self, tx: Arc<Transaction>) {
        self.pending.push(tx);
    }
}
