use std::sync::Arc;

use alloy_consensus::Transaction as AlloyTransactionTrait;
use revm_primitives::{Address, B256, U256};

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
    pub fn new(current: Option<SimulatedTx>, pending: &TxList) -> SimulatedTxList {
        let mut pending = pending.clone();

        // Remove current from pending, if it exists
        if let Some(ref current) = current {
            if pending.peek_nonce().is_some_and(|nonce| current.nonce() == nonce) {
                pending.pop_front();
            }

            debug_assert!(
                pending.peek_nonce().map_or(true, |nonce| current.nonce() == nonce + 1),
                "pending tx list nonce must be consecutive from current"
            );
        }

        SimulatedTxList { current, pending }
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

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn hash(&self) -> B256 {
        self.current.as_ref().map(|t| t.tx_hash()).unwrap_or_else(|| self.pending.tx_hash())
    }

    /// Removes the active transaction for the sender from the list.
    /// Returns true if all transactions for this sender have now been applied.
    pub fn pop(&mut self, base_fee: u64) -> bool {
        debug_assert!(self.current.is_some(), "Tried popping on a SimulatedTxList with current None: {self:#?}");
        if self.pending.is_empty() {
            return true;
        }
        if let Some(nonce) = self.current.take().map(|t| t.nonce()) {
            self.pending.first_ready(nonce + 1, base_fee).is_none()
        } else {
            self.pending.peek().is_some_and(|t| t.max_fee_per_gas() as u64 > base_fee)
        }
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

    pub fn nonce(&self) -> u64 {
        if let Some(tx) = &self.current {
            tx.nonce()
        } else {
            self.pending.peek_nonce().unwrap_or_default()
        }
    }

    pub fn push(&mut self, tx: Arc<Transaction>) {
        self.pending.push(tx);
    }

    #[inline]
    pub fn weight(&self) -> U256 {
        if let Some(tx) = &self.current {
            return tx.payment;
        }
        if let Some(tx) = self.pending.peek() {
            if tx.is_deposit() {
                return U256::MAX;
            } 
            return U256::from(tx.priority_fee_or_price());
        }
        U256::ZERO
    }

    pub fn payment(&self) -> alloy_primitives::Uint<256, 4> {
        self.current.as_ref().map(|c| c.payment).unwrap_or_default()
    }
}

impl From<SimulatedTx> for SimulatedTxList {
    fn from(current: SimulatedTx) -> Self {
        Self { current: Some(current), pending: TxList::default() }
    }
}
