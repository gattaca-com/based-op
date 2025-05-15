//! Optimism transaction types

pub mod signed;
pub mod tx_type;

use alloy_primitives::Address;
use auto_impl::auto_impl;

/// Trait for accessing sender information from a transaction.
/// Used by pools.
#[auto_impl(&, Box, Arc)]
pub trait TransactionSenderInfo {
    /// Returns the sender address of the transaction.
    fn sender(&self) -> Address;
    /// Returns the nonce of the transaction.
    fn nonce(&self) -> u64;
}

mod tx_type;

/// Kept for concistency tests
#[cfg(test)]
mod signed;

pub use op_alloy_consensus::{OpTxType, OpTypedTransaction};

/// Signed transaction.
pub type OpTransactionSigned = op_alloy_consensus::OpTxEnvelope;

/// A trait that represents an optimism transaction, mainly used to indicate whether or not the
/// transaction is a deposit transaction.
pub trait OpTransaction {
    /// Whether or not the transaction is a dpeosit transaction.
    fn is_deposit(&self) -> bool;
}

impl OpTransaction for op_alloy_consensus::OpTxEnvelope {
    fn is_deposit(&self) -> bool {
        Self::is_deposit(self)
    }
}
