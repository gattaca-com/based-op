use std::sync::Arc;

use alloy_primitives::U256;

use crate::transaction::{SimulatedTx, SimulatedTxList, Transaction};

pub mod bundle;
pub use bundle::{SimulatedBundle, ValidatedBundle};

/// An order is either a transaction or an atomic bundle of transactions.
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

impl From<Arc<Transaction>> for Order {
    fn from(tx: Arc<Transaction>) -> Self {
        Order::Tx(tx)
    }
}

impl From<Arc<ValidatedBundle>> for Order {
    fn from(bundle: Arc<ValidatedBundle>) -> Self {
        Order::Bundle(bundle)
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

#[derive(Debug, Clone)]
pub enum SimulatedOrder {
    Tx(SimulatedTx),
    Bundle(SimulatedBundle),
}

/// An order that is ready to be executed in the next block.
#[derive(Debug, Clone)]
pub enum PendingOrder {
    Tx(SimulatedTxList),
    Bundle(SimulatedBundle),
}

impl PendingOrder {
    pub fn as_tx_list(&self) -> Option<&SimulatedTxList> {
        match self {
            PendingOrder::Tx(list) => Some(list),
            _ => None,
        }
    }

    pub fn as_tx_list_mut(&mut self) -> Option<&mut SimulatedTxList> {
        match self {
            PendingOrder::Tx(list) => Some(list),
            _ => None,
        }
    }

    pub fn as_bundle(&self) -> Option<&SimulatedBundle> {
        match self {
            PendingOrder::Bundle(bundle) => Some(bundle),
            _ => None,
        }
    }

    pub fn weight(&self) -> U256 {
        match self {
            PendingOrder::Tx(list) => list.weight(),
            PendingOrder::Bundle(bundle) => bundle.weight(),
        }
    }
}
