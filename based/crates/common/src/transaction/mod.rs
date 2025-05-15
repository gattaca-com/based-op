pub mod simulated;
pub mod tx_list;

use std::{ops::Deref, sync::Arc};

use alloy_consensus::{SignableTransaction, Transaction as TransactionTrait, TxEip1559};
use alloy_eips::eip2718::{Decodable2718, Encodable2718};
use alloy_primitives::{Address, B256, Bytes, U256};
use op_alloy_consensus::{DepositTransaction, OpTxEnvelope};
use op_revm::OpTransaction;
use reth_evm::IntoTxEnv;
use reth_optimism_primitives::OpTransactionSigned;
use reth_primitives_traits::SignedTransaction;
use revm::context::TxEnv;
pub use simulated::{SimulatedTx, SimulatedTxList};
pub use tx_list::TxList;

use crate::{signing::ECDSASigner, typedefs::BlockSyncMessage};

#[derive(Clone, Debug)]
pub struct Transaction {
    pub tx: OpTxEnvelope,
    /// The sender of the transaction.
    /// Recovered from the tx on initialisation.
    sender: Address,
    pub envelope: Bytes,
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
        self.gas_price_or_max_fee().is_none_or(|price| price > base_fee as u128)
    }

    #[inline]
    pub fn fill_tx_env(&self, tx: &mut OpTransaction<TxEnv>) {
        tx.enveloped_tx = Some(self.encode());
        let tx_env = &mut tx.base;
        tx_env.caller = self.sender;
        match &self.tx {
            OpTxEnvelope::Legacy(tx) => {
                tx_env.gas_limit = tx.tx().gas_limit;
                tx_env.gas_price = tx.tx().gas_price;
                tx_env.gas_priority_fee = None;
                tx_env.value = tx.tx().value;
                tx_env.kind = tx.tx().to;
                tx_env.data = tx.tx().input.clone();
                tx_env.chain_id = tx.tx().chain_id;
                tx_env.nonce = tx.tx().nonce;
                tx_env.access_list.0.clear();
                tx_env.blob_hashes.clear();
                tx_env.max_fee_per_blob_gas = 0;
                tx_env.authorization_list = vec![];
            }
            OpTxEnvelope::Eip2930(tx) => {
                tx_env.gas_limit = tx.tx().gas_limit;
                tx_env.gas_price = tx.tx().gas_price;
                tx_env.gas_priority_fee = None;
                tx_env.kind = tx.tx().to;
                tx_env.value = tx.tx().value;
                tx_env.data = tx.tx().input.clone();
                tx_env.chain_id = Some(tx.tx().chain_id);
                tx_env.nonce = tx.tx().nonce;
                tx_env.access_list = tx.tx().access_list.clone();
                tx_env.blob_hashes.clear();
                tx_env.max_fee_per_blob_gas = 0;
                tx_env.authorization_list = vec![];
            }
            OpTxEnvelope::Eip1559(tx) => {
                tx_env.gas_limit = tx.tx().gas_limit;
                tx_env.gas_price = tx.tx().max_fee_per_gas;
                tx_env.gas_priority_fee = Some(tx.tx().max_priority_fee_per_gas);
                tx_env.kind = tx.tx().to;
                tx_env.value = tx.tx().value;
                tx_env.data = tx.tx().input.clone();
                tx_env.chain_id = Some(tx.tx().chain_id);
                tx_env.nonce = tx.tx().nonce;
                tx_env.access_list = tx.tx().access_list.clone();
                tx_env.blob_hashes.clear();
                tx_env.max_fee_per_blob_gas = 0;
                tx_env.authorization_list = vec![];
            }
            OpTxEnvelope::Eip7702(tx) => {
                tx_env.gas_limit = tx.tx().gas_limit;
                tx_env.gas_price = tx.tx().max_fee_per_gas;
                tx_env.gas_priority_fee = Some(tx.tx().max_priority_fee_per_gas);
                tx_env.kind = tx.tx().to.into();
                tx_env.value = tx.tx().value;
                tx_env.data = tx.tx().input.clone();
                tx_env.chain_id = Some(tx.tx().chain_id);
                tx_env.nonce = tx.tx().nonce;
                tx_env.access_list = tx.tx().access_list.clone();
                tx_env.blob_hashes.clear();
                tx_env.max_fee_per_blob_gas = 0;
                tx_env.authorization_list = tx.tx().authorization_list.clone();
            }
            OpTxEnvelope::Deposit(deposit_tx) => {
                tx_env.access_list.0.clear();
                tx_env.gas_limit = deposit_tx.gas_limit;
                tx_env.gas_price = 0;
                tx_env.gas_priority_fee = None;
                tx_env.kind = deposit_tx.to;
                tx_env.value = deposit_tx.value;
                tx_env.data = deposit_tx.input.clone();
                tx_env.chain_id = None;
                tx_env.nonce = 0;
                tx_env.authorization_list = vec![];
                tx.deposit.source_hash = deposit_tx.source_hash;
                tx.deposit.mint = deposit_tx.mint;
                tx.deposit.is_system_transaction = deposit_tx.is_system_transaction;
                return;
            }
        }
    }

    // #[inline]
    // pub fn random() -> Self {
    //     let value = 50;
    //     let max_gas_units = 50;
    //     let max_fee_per_gas = 50;
    //     let nonce = 1;
    //     let chain_id = 1000;
    //     let max_priority_fee_per_gas = 1000;

    //     let signing_wallet = ECDSASigner::try_from_secret(B256::random().as_ref()).unwrap();
    //     let from = Address::random();
    //     let to = Address::random();
    //     let value = U256::from_limbs([value, 0, 0, 0]);
    //     let tx = TxEip1559 {
    //         chain_id,
    //         nonce,
    //         gas_limit: max_gas_units,
    //         max_fee_per_gas,
    //         max_priority_fee_per_gas,
    //         to: TxKind::Call(to),
    //         value,
    //         ..Default::default()
    //     };
    //     let signed_tx = signing_wallet.sign_tx(tx).unwrap();
    //     let tx = OpTxEnvelope::Eip1559(signed_tx);
    //     let envelope = tx.encoded_2718().into();
    //     Self { sender: from, tx, envelope }
    // }

    pub fn decode(bytes: Bytes) -> Result<Self, alloy_rlp::Error> {
        let tx = OpTxEnvelope::decode_2718(&mut bytes.as_ref())?;

        let sender = match &tx {
            OpTxEnvelope::Legacy(signed) => signed.recover_signer().unwrap(),
            OpTxEnvelope::Eip2930(signed) => signed.recover_signer().unwrap(),
            OpTxEnvelope::Eip1559(signed) => signed.recover_signer().unwrap(),
            OpTxEnvelope::Eip7702(signed) => signed.recover_signer().unwrap(),
            OpTxEnvelope::Deposit(sealed) => sealed.from,
            _ => panic!("invalid tx type"),
        };

        Ok(Self { sender, tx, envelope: bytes })
    }

    pub fn encode(&self) -> Bytes {
        debug_assert_eq!(self.envelope, self.tx.encoded_2718());
        self.envelope.clone()
    }

    pub fn from_block(block: &BlockSyncMessage) -> Vec<Arc<Transaction>> {
        block.transactions_with_sender().map(|(_, t)| Arc::new(t.clone().into())).collect()
    }

    pub fn to_op_tx_env(&self) -> OpTransaction<TxEnv> {
        let mut op_env = OpTransaction::default();
        self.fill_tx_env(&mut op_env);
        op_env
    }
}

impl Deref for Transaction {
    type Target = OpTxEnvelope;

    fn deref(&self) -> &Self::Target {
        &self.tx
    }
}

// impl From<&Transaction> for OptimismFields {
//     fn from(value: &Transaction) -> Self {
//         let envelope = value.envelope.clone();
//         if let OpTxEnvelope::Deposit(tx) = &value.tx {
//             Self {
//                 source_hash: tx.source_hash(),
//                 mint: tx.mint(),
//                 is_system_transaction: Some(tx.is_system_transaction()),
//                 enveloped_tx: Some(envelope),
//             }
//         } else {
//             Self { source_hash: None, mint: None, is_system_transaction: Some(false), enveloped_tx: Some(envelope) }
//         }
//     }
// }

impl From<OpTransactionSigned> for Transaction {
    fn from(value: OpTransactionSigned) -> Self {
        let sender = value.recover_signer().expect("could not recover signer");
        let envelope = value.encoded_2718().into();
        Self { tx: value.into(), sender, envelope }
    }
}
