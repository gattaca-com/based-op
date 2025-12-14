use std::sync::Arc;

use crate::transaction::Transaction;

pub mod bundle;
use bundle::ValidatedBundle;

/// An order is either a transaction or an atomic bundle of transactions. They are the basic building blocks
/// of a block, and used as such in building algorithms.
#[derive(Debug, Clone)]
pub enum Order {
    Tx(Arc<Transaction>),
    Bundle(Arc<ValidatedBundle>),
}

impl From<Transaction> for Order {
    fn from(tx: Transaction) -> Self {
        Order::Tx(Arc::new(tx))
    }
}

impl From<ValidatedBundle> for Order {
    fn from(bundle: ValidatedBundle) -> Self {
        Order::Bundle(Arc::new(bundle))
    }
}

// TODO(mempirate): Implement common methods for all orders.
impl Order {
    pub fn tx(&self) -> Option<&Arc<Transaction>> {
        match self {
            Order::Tx(tx) => Some(tx),
            _ => None,
        }
    }

    pub fn bundle(&self) -> Option<&Arc<ValidatedBundle>> {
        match self {
            Order::Bundle(bundle) => Some(bundle),
            _ => None,
        }
    }
}
