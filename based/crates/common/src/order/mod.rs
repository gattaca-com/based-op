use crate::transaction::Transaction;

pub mod bundle;
use bundle::ValidatedBundle;

/// An order is either a transaction or an atomic bundle of transactions. They are the basic building blocks
/// of a block, and used as such in building algorithms.
#[derive(Debug)]
pub enum Order {
    Tx(Transaction),
    Bundle(ValidatedBundle),
}

impl From<Transaction> for Order {
    fn from(tx: Transaction) -> Self {
        Order::Tx(tx)
    }
}

impl From<ValidatedBundle> for Order {
    fn from(bundle: ValidatedBundle) -> Self {
        Order::Bundle(bundle)
    }
}

// TODO: Implement common methods for all orders.
impl Order {}
