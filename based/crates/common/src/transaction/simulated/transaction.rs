use std::{fmt::Debug, ops::Deref, sync::Arc};

use alloy_consensus::{Eip658Value, Receipt, Transaction as TransactionTrait, TxReceipt};
use alloy_primitives::U256;
use alloy_rpc_types::TransactionReceipt;
use op_alloy_consensus::{OpDepositReceipt, OpDepositReceiptWithBloom, OpReceiptEnvelope, OpTxType};
use op_alloy_rpc_types::{L1BlockInfo, OpTransactionReceipt};
use reth_optimism_primitives::{transaction::TransactionSenderInfo, OpReceipt};
use reth_primitives::ReceiptWithBloom;
use revm_primitives::{Address, EvmState, ResultAndState};

use crate::transaction::Transaction;

#[derive(Clone, Debug)]
pub struct SimulatedTx {
    /// original tx
    pub tx: Arc<Transaction>,
    /// revm execution result. Contains gas_used, logs, output, etc.
    pub result_and_state: ResultAndState,
    /// Coinbase balance diff, after_sim - before_sim
    pub payment: U256,
    /// Cache the depositor account prior to the state transition for the deposit nonce.
    /// Note: this is only used for deposit transactions.
    pub deposit_nonce: Option<u64>,
}

impl SimulatedTx {
    pub fn new(
        tx: Arc<Transaction>,
        result_and_state: ResultAndState,
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
                authorization_list: None,
            },
            l1_block_info: L1BlockInfo::default(),
        }
    }

    pub fn gas_used(&self) -> u64 {
        self.result_and_state.result.gas_used()
    }
}

impl AsRef<ResultAndState> for SimulatedTx {
    fn as_ref(&self) -> &ResultAndState {
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
