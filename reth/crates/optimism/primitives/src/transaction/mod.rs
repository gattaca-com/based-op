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
