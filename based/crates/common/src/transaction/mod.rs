pub mod simulated;
pub mod tx_list;

use alloy_consensus::{Transaction as TransactionTrait, TxEip1559};
use alloy_eips::eip2718::Decodable2718;
use alloy_primitives::{Address, Bytes, B256, U256};
use op_alloy_consensus::OpTxEnvelope;
use revm_primitives::TxKind;
pub use simulated::{SimulatedTx, SimulatedTxList};
pub use tx_list::TxList;

use crate::signing::ECDSASigner;

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
        0
    }

    #[inline]
    pub fn nonce_ref(&self) -> &u64 {
        todo!()
    }

    #[inline]
    pub fn random() -> Self {
        let value = 50;
        let max_gas_units = 50;
        let max_fee_per_gas = 50;
        let nonce = 1;
        let chain_id = 1000;
        let max_priority_fee_per_gas = 1000;

        let signing_wallet = ECDSASigner::try_from_secret(B256::random().as_ref()).unwrap();
        let from = Address::random();
        let to = Address::random();
        let value = U256::from_limbs([value, 0, 0, 0]);
        let tx = TxEip1559 {
            chain_id,
            nonce,
            gas_limit: max_gas_units,
            max_fee_per_gas,
            max_priority_fee_per_gas,
            to: TxKind::Call(to),
            value,
            ..Default::default()
        };
        let signed_tx = signing_wallet.sign_tx(tx).unwrap();
        Self { sender: from, tx: OpTxEnvelope::Eip1559(signed_tx) }
    }

    pub fn decode(bytes: Bytes) -> Result<Self, alloy_rlp::Error> {
        let tx = OpTxEnvelope::decode_2718(&mut bytes.as_ref())?;

        let sender = match &tx {
            OpTxEnvelope::Legacy(signed) => signed.recover_signer().unwrap(),
            OpTxEnvelope::Eip2930(signed) => signed.recover_signer().unwrap(),
            OpTxEnvelope::Eip1559(signed) => signed.recover_signer().unwrap(),
            OpTxEnvelope::Eip7702(signed) => signed.recover_signer().unwrap(),
            OpTxEnvelope::Deposit(_sealed) => Address::ZERO,
            _ => panic!("invalid tx type"),
        };

        Ok(Self { sender, tx })
    }
}
