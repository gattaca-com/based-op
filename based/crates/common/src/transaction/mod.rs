pub mod simulated;
pub mod tx_list;

use alloy_consensus::Transaction as TransactionTrait;
use alloy_primitives::{Address, B256};
use op_alloy_consensus::OpTxEnvelope;
pub use simulated::{SimulatedTx, SimulatedTxList};
pub use tx_list::TxList;

#[derive(Clone, Debug)]
pub struct Transaction {
    pub tx: OpTxEnvelope,
    /// The sender of the transaction.
    /// Recovered from the tx on initialisation.
    sender: Address,
}

impl Transaction {
    #[inline]
    pub fn sender(&self) -> Address {
        self.sender
    }

    #[inline]
    pub fn sender_ref(&self) -> &Address {
        &self.sender
    }

    #[inline]
    pub fn hash(&self) -> B256 {
        self.tx.tx_hash()
    }

    /// Returns the gas price for type 0 and 1 transactions.
    /// Returns the max fee for EIP-1559 transactions.
    /// Returns `None` for deposit transactions.
    #[inline]
    pub fn gas_price_or_max_fee(&self) -> Option<u128> {
        match &self.tx {
            OpTxEnvelope::Legacy(tx) => Some(tx.tx().gas_price),
            OpTxEnvelope::Eip2930(tx) => Some(tx.tx().gas_price),
            OpTxEnvelope::Eip1559(tx) => Some(tx.tx().max_fee_per_gas),
            OpTxEnvelope::Eip7702(tx) => Some(tx.tx().max_fee_per_gas),
            OpTxEnvelope::Deposit(_) => None,
            _ => unreachable!(),
        }
    }

    #[inline]
    pub fn effective_gas_price(&self, base_fee: u64) -> u128 {
        self.tx.effective_gas_price(Some(base_fee))
    }

    #[inline]
    pub fn nonce(&self) -> u64 {
        todo!()
    }

    #[inline]
    pub fn nonce_ref(&self) -> &u64 {
        todo!()
    }
}
