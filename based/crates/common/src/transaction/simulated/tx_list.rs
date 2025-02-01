use std::{ops::Deref, sync::Arc};

use revm_primitives::{Address, B256};

use crate::transaction::{simulated::transaction::SimulatedTx, Transaction, TxList};

/// A list of simulated transactions from a single sender.
/// nonce-sorted, i.e. txs[0].nonce = state[address].nonce + 1.
/// First is Simulated Top Of Block
#[derive(Clone, Debug)]
pub struct SimulatedTxList {
    pub current: Option<SimulatedTx>,
    pub pending: TxList,
}

impl SimulatedTxList {
    pub fn new(current: SimulatedTx, pending: TxList) -> SimulatedTxList {
        SimulatedTxList { current: Some(current), pending }
    }

    pub fn len(&self) -> usize {
        self.pending.len() + self.current.is_some() as usize
    }

    pub fn hash(&self) -> B256 {
        self.current.as_ref().map(|t| t.tx_hash()).unwrap_or_else(|| self.pending.tx_hash())
    }

    /// Removes the active transaction for the sender from the list.
    /// Returns true if all transactions for this sender have now been applied.
    pub fn pop(&mut self) -> bool {
        debug_assert!(self.current.is_some(), "Tried popping on a SimulatedTxList with current None: {self:#?}");
        self.current = None;

        self.pending.is_empty()
    }

    pub fn sender(&self) -> Address {
        if let Some(tx) = &self.current {
            tx.tx.sender
        } else {
            self.pending.front().as_ref().map(|t| t.sender).unwrap_or_default()
        }
    }
}
