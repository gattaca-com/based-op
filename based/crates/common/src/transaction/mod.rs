pub mod simulated;
pub mod tx_list;

use std::{ops::Deref, sync::Arc};

use alloy_consensus::{SignableTransaction, Transaction as TransactionTrait, TxEip1559};
use alloy_eips::eip2718::{Decodable2718, Encodable2718};
use alloy_primitives::{Address, Bytes, B256, U256};
use op_alloy_consensus::{DepositTransaction, OpTxEnvelope};
use reth_optimism_primitives::{transaction::TransactionSenderInfo, OpTransactionSigned};
use reth_primitives_traits::SignedTransaction;
use revm_primitives::{OptimismFields, TxEnv, TxKind};
pub use simulated::{SimulatedTx, SimulatedTxList};
pub use tx_list::TxList;

use crate::{communication::messages::BlockSyncMessage, signing::ECDSASigner};

#[derive(Clone, Debug)]
pub struct Transaction {
    pub tx: OpTxEnvelope,
    /// The sender of the transaction.
    /// Recovered from the tx on initialisation.
    sender: Address,
    envelope: Bytes,
}

impl Transaction {
    pub fn new(tx: OpTxEnvelope, sender: Address, envelope: Bytes) -> Self {
        Self { tx, sender, envelope }
    }

    #[inline]
    pub fn sender(&self) -> Address {
        self.sender
    }

    #[inline]
    pub fn sender_ref(&self) -> &Address {
        &self.sender
    }

    #[inline]
    pub fn nonce_ref(&self) -> &u64 {
        match &self.tx {
            OpTxEnvelope::Legacy(tx) => &tx.tx().nonce,
            OpTxEnvelope::Eip2930(tx) => &tx.tx().nonce,
            OpTxEnvelope::Eip1559(tx) => &tx.tx().nonce,
            OpTxEnvelope::Eip7702(tx) => &tx.tx().nonce,
            OpTxEnvelope::Deposit(_) => &0,
            _ => unreachable!(),
        }
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

    /// Returns true if the transaction is valid for a block with the given base fee.
    #[inline]
    pub fn valid_for_block(&self, base_fee: u64) -> bool {
        self.gas_price_or_max_fee().map_or(false, |price| price < base_fee as u128)
    }

    #[inline]
    pub fn fill_tx_env(&self, env: &mut TxEnv) {
        env.caller = self.sender;
        env.gas_limit = self.gas_limit();
        env.gas_price = U256::from(self.max_fee_per_gas());
        env.gas_priority_fee = self.max_priority_fee_per_gas().map(U256::from);
        env.transact_to = self.to().into();
        env.value = self.value();
        env.data = self.input().clone();
        env.chain_id = self.chain_id();
        env.nonce = Some(self.nonce());
        env.access_list = self.access_list().cloned().unwrap_or_default().0;
        env.blob_hashes = self.blob_versioned_hashes().map(|t| t.to_vec()).unwrap_or_default();
        env.max_fee_per_blob_gas = self.max_fee_per_blob_gas().map(U256::from);
        env.optimism = self.into()
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
        let tx = OpTxEnvelope::Eip1559(signed_tx);
        let envelope = tx.encoded_2718().into();
        Self { sender: from, tx, envelope }
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

        Ok(Self { sender, tx, envelope: bytes })
    }

    pub fn encode(&self) -> Bytes {
        self.tx.encoded_2718().into()
    }

    pub fn from_block(block: &BlockSyncMessage) -> Vec<Arc<Transaction>> {
        block.body.transactions.iter().map(|t| Arc::new(t.clone().into())).collect()
    }
}

impl Deref for Transaction {
    type Target = OpTxEnvelope;

    fn deref(&self) -> &Self::Target {
        &self.tx
    }
}

impl From<&Transaction> for OptimismFields {
    fn from(value: &Transaction) -> Self {
        let envelope = value.envelope.clone();
        if let OpTxEnvelope::Deposit(tx) = &value.tx {
            Self {
                source_hash: tx.source_hash(),
                mint: tx.mint(),
                is_system_transaction: Some(tx.is_system_transaction()),
                enveloped_tx: Some(envelope),
            }
        } else {
            Self { source_hash: None, mint: None, is_system_transaction: Some(false), enveloped_tx: Some(envelope) }
        }
    }
}

impl From<OpTransactionSigned> for Transaction {
    fn from(value: OpTransactionSigned) -> Self {
        let sender = value.recover_signer().expect("could not recover signer");
        let envelope = value.encoded_2718().into();
        let signature = value.signature;
        let tx = match value.transaction {
            op_alloy_consensus::OpTypedTransaction::Legacy(tx_legacy) => {
                OpTxEnvelope::Legacy(tx_legacy.into_signed(signature))
            }
            op_alloy_consensus::OpTypedTransaction::Eip2930(tx_eip2930) => {
                OpTxEnvelope::Eip2930(tx_eip2930.into_signed(signature))
            }
            op_alloy_consensus::OpTypedTransaction::Eip1559(tx_eip1559) => {
                OpTxEnvelope::Eip1559(tx_eip1559.into_signed(signature))
            }
            op_alloy_consensus::OpTypedTransaction::Eip7702(tx_eip7702) => {
                OpTxEnvelope::Eip7702(tx_eip7702.into_signed(signature))
            }
            op_alloy_consensus::OpTypedTransaction::Deposit(tx_deposit) => OpTxEnvelope::Deposit(tx_deposit.seal()),
        };
        Self { tx, sender, envelope }
    }
}

impl TransactionSenderInfo for Transaction {
    #[inline]
    fn sender(&self) -> Address {
        self.sender
    }

    #[inline]
    fn nonce(&self) -> u64 {
        self.tx.nonce()
    }
}
