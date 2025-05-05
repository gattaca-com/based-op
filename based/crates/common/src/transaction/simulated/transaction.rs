use std::{fmt::Debug, ops::Deref, sync::Arc};

use alloy_consensus::{Eip658Value, Receipt, Transaction as TransactionTrait, TxReceipt};
use alloy_eips::{Typed2718, eip7702::SignedAuthorization};
use alloy_primitives::{ChainId, U256};
use alloy_rpc_types::{AccessList, TransactionReceipt};
use op_alloy_consensus::{OpDepositReceipt, OpDepositReceiptWithBloom, OpReceiptEnvelope, OpTxType};
use op_alloy_rpc_types::{L1BlockInfo, OpTransactionReceipt};
use op_revm::OpHaltReason;
use reth_optimism_primitives::{OpReceipt, transaction::TransactionSenderInfo};
use reth_primitives::ReceiptWithBloom;
use reth_primitives_traits::SignedTransaction;
use revm::{context::result::ResultAndState, state::EvmState};
use revm_primitives::{Address, B256, Bytes, TxKind};

use crate::transaction::Transaction;

#[derive(Clone, Debug)]
pub struct SimulatedTx {
    /// original tx
    pub tx: Arc<Transaction>,
    /// revm execution result. Contains gas_used, logs, output, etc.
    pub result_and_state: ResultAndState<OpHaltReason>,
    /// Coinbase balance diff, after_sim - before_sim
    pub payment: U256,
    /// Cache the depositor account prior to the state transition for the deposit nonce.
    /// Note: this is only used for deposit transactions.
    pub deposit_nonce: Option<u64>,
}

impl SimulatedTx {
    pub fn new(
        tx: Arc<Transaction>,
        result_and_state: ResultAndState<OpHaltReason>,
        payment: U256,
        deposit_nonce: Option<u64>,
    ) -> Self {
        Self { tx, result_and_state, payment, deposit_nonce }
    }

    pub fn take_state(&mut self) -> EvmState {
        if cfg!(debug_assertions) {
            self.result_and_state.state.clone()
        } else {
            std::mem::take(&mut self.result_and_state.state)
        }
    }

    pub fn clone_state(&self) -> EvmState {
        self.result_and_state.state.clone()
    }

    pub fn receipt(&self, cumulative_gas_used: u64, canyon_active: bool) -> ReceiptWithBloom<OpReceipt> {
        let receipt = Receipt {
            logs: self.result_and_state.result.logs().to_owned(),
            cumulative_gas_used,
            status: alloy_consensus::Eip658Value::Eip658(self.result_and_state.result.is_success()),
        };
        let receipt = match self.tx.tx_type() {
            OpTxType::Legacy => OpReceipt::Legacy(receipt),
            OpTxType::Eip2930 => OpReceipt::Eip2930(receipt),
            OpTxType::Eip1559 => OpReceipt::Eip1559(receipt),
            OpTxType::Eip7702 => OpReceipt::Eip7702(receipt),
            OpTxType::Deposit => OpReceipt::Deposit(OpDepositReceipt {
                inner: receipt,
                deposit_nonce: self.deposit_nonce,
                // The deposit receipt version was introduced in Canyon to indicate an update to
                // how receipt hashes should be computed when set. The state
                // transition process ensures this is only set for
                // post-Canyon deposit transactions.
                deposit_receipt_version: (self.tx.is_deposit() && canyon_active).then_some(1),
            }),
        };
        receipt.into_with_bloom()
    }

    pub fn op_tx_receipt(
        &self,
        cumulative_gas_used: u64,
        block_number: u64,
        block_timestamp: u64,
        base_fee: u64,
        tx_id: u64,
    ) -> OpTransactionReceipt {
        let hash = self.tx_hash();
        let logs_bloom = alloy_primitives::logs_bloom(self.result_and_state.result.logs().iter());
        let logs = self
            .result_and_state
            .result
            .logs()
            .iter()
            .enumerate()
            .map(|(i, t)| alloy_rpc_types::Log {
                inner: t.clone(),
                block_hash: None,
                block_number: Some(block_number),
                block_timestamp: Some(block_timestamp),
                transaction_hash: Some(hash),
                transaction_index: Some(tx_id),
                log_index: Some(i as u64),
                removed: false,
            })
            .collect();

        let inner_receipt = Receipt { status: Eip658Value::Eip658(true), cumulative_gas_used, logs };
        let receipt = match self.tx.tx_type() {
            OpTxType::Legacy => OpReceiptEnvelope::Legacy(ReceiptWithBloom { receipt: inner_receipt, logs_bloom }),
            OpTxType::Eip2930 => OpReceiptEnvelope::Eip2930(ReceiptWithBloom { receipt: inner_receipt, logs_bloom }),
            OpTxType::Eip1559 => OpReceiptEnvelope::Eip1559(ReceiptWithBloom { receipt: inner_receipt, logs_bloom }),
            OpTxType::Eip7702 => OpReceiptEnvelope::Eip7702(ReceiptWithBloom { receipt: inner_receipt, logs_bloom }),
            OpTxType::Deposit => {
                let inner = OpDepositReceiptWithBloom {
                    receipt: OpDepositReceipt {
                        inner: inner_receipt,
                        deposit_nonce: self.deposit_nonce,
                        deposit_receipt_version: None,
                    },
                    logs_bloom,
                };
                OpReceiptEnvelope::Deposit(inner)
            }
        };
        OpTransactionReceipt {
            inner: TransactionReceipt {
                inner: receipt,
                transaction_hash: hash,
                transaction_index: Some(tx_id),
                block_hash: None,
                block_number: Some(block_number),
                gas_used: self.gas_used(),
                effective_gas_price: self.effective_gas_price(Some(base_fee)),
                blob_gas_used: Some(0),
                blob_gas_price: Some(0),
                from: self.sender(),
                to: self.to(),
                contract_address: None,
            },
            l1_block_info: L1BlockInfo::default(),
        }
    }

    pub fn gas_used(&self) -> u64 {
        self.result_and_state.result.gas_used()
    }
}

impl AsRef<ResultAndState<OpHaltReason>> for SimulatedTx {
    fn as_ref(&self) -> &ResultAndState<OpHaltReason> {
        &self.result_and_state
    }
}
impl Deref for SimulatedTx {
    type Target = Arc<Transaction>;

    fn deref(&self) -> &Self::Target {
        &self.tx
    }
}

impl TransactionSenderInfo for SimulatedTx {
    fn sender(&self) -> Address {
        self.sender
    }

    fn nonce(&self) -> u64 {
        self.tx.nonce()
    }
}

impl Typed2718 for SimulatedTx {
    #[doc = " Returns the EIP-2718 type flag."]
    fn ty(&self) -> u8 {
        self.tx.ty()
    }
}

impl TransactionTrait for SimulatedTx {
    #[doc = " Get `chain_id`."]
    fn chain_id(&self) -> Option<ChainId> {
        self.tx.chain_id()
    }

    #[doc = " Get `nonce`."]
    fn nonce(&self) -> u64 {
        self.tx.nonce()
    }

    #[doc = " Get `gas_limit`."]
    fn gas_limit(&self) -> u64 {
        self.tx.gas_limit()
    }

    #[doc = " Get `gas_price`."]
    fn gas_price(&self) -> Option<u128> {
        self.tx.gas_price()
    }

    #[doc = " For dynamic fee transactions returns the maximum fee per gas the caller is willing to pay."]
    #[doc = ""]
    #[doc = " For legacy fee transactions this is `gas_price`."]
    #[doc = ""]
    #[doc = " This is also commonly referred to as the \"Gas Fee Cap\"."]
    fn max_fee_per_gas(&self) -> u128 {
        self.tx.max_fee_per_gas()
    }

    #[doc = " For dynamic fee transactions returns the Priority fee the caller is paying to the block"]
    #[doc = " author."]
    #[doc = ""]
    #[doc = " This will return `None` for legacy fee transactions"]
    fn max_priority_fee_per_gas(&self) -> Option<u128> {
        self.tx.max_priority_fee_per_gas()
    }

    #[doc = " Max fee per blob gas for EIP-4844 transaction."]
    #[doc = ""]
    #[doc = " Returns `None` for non-eip4844 transactions."]
    #[doc = ""]
    #[doc = " This is also commonly referred to as the \"Blob Gas Fee Cap\"."]
    fn max_fee_per_blob_gas(&self) -> Option<u128> {
        self.tx.max_fee_per_blob_gas()
    }

    #[doc = " Return the max priority fee per gas if the transaction is an dynamic fee transaction, and"]
    #[doc = " otherwise return the gas price."]
    #[doc = ""]
    #[doc = " # Warning"]
    #[doc = ""]
    #[doc = " This is different than the `max_priority_fee_per_gas` method, which returns `None` for"]
    #[doc = " legacy fee transactions."]
    fn priority_fee_or_price(&self) -> u128 {
        self.tx.priority_fee_or_price()
    }

    #[doc = " Returns the effective gas price for the given base fee."]
    #[doc = ""]
    #[doc = " If the transaction is a legacy fee transaction, the gas price is returned."]
    fn effective_gas_price(&self, base_fee: Option<u64>) -> u128 {
        self.tx.effective_gas_price(base_fee)
    }

    #[doc = " Returns `true` if the transaction supports dynamic fees."]
    fn is_dynamic_fee(&self) -> bool {
        self.tx.is_dynamic_fee()
    }

    #[doc = " Returns the transaction kind."]
    fn kind(&self) -> TxKind {
        self.tx.kind()
    }

    #[doc = " Returns true if the transaction is a contract creation."]
    #[doc = " We don\'t provide a default implementation via `kind` as it copies the 21-byte"]
    #[doc = " [`TxKind`] for this simple check. A proper implementation shouldn\'t allocate."]
    fn is_create(&self) -> bool {
        self.tx.is_create()
    }

    #[doc = " Get `value`."]
    fn value(&self) -> U256 {
        self.tx.value()
    }

    #[doc = " Get `data`."]
    fn input(&self) -> &Bytes {
        self.tx.input()
    }

    #[doc = " Returns the EIP-2930 `access_list` for the particular transaction type. Returns `None` for"]
    #[doc = " older transaction types."]
    fn access_list(&self) -> Option<&AccessList> {
        self.tx.access_list()
    }

    #[doc = " Blob versioned hashes for eip4844 transaction. For previous transaction types this is"]
    #[doc = " `None`."]
    fn blob_versioned_hashes(&self) -> Option<&[B256]> {
        self.tx.blob_versioned_hashes()
    }

    #[doc = " Returns the [`SignedAuthorization`] list of the transaction."]
    #[doc = ""]
    #[doc = " Returns `None` if this transaction is not EIP-7702."]
    fn authorization_list(&self) -> Option<&[SignedAuthorization]> {
        self.tx.authorization_list()
    }
}

impl reth_optimism_primitives::transaction::signed::OpTransaction for SimulatedTx {
    fn is_deposit(&self) -> bool {
        false
    }
}
