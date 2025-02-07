use std::{fmt::Debug, ops::Deref, sync::Arc};

use alloy_consensus::{Receipt, TxReceipt};
use alloy_primitives::U256;
use op_alloy_consensus::{OpDepositReceipt, OpTxType};
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
    deposit_nonce: Option<u64>,
}

impl SimulatedTx {
    pub fn new(
        tx: Arc<Transaction>,
        result_and_state: ResultAndState,
        start_balance: U256,
        coinbase: Address,
        deposit_nonce: Option<u64>,
    ) -> Self {
        // Determing payment
        let end_balance = result_and_state.state.get(&coinbase).map(|a| a.info.balance).unwrap_or_default();
        let payment = end_balance.saturating_sub(start_balance);

        Self { tx, result_and_state, payment, deposit_nonce }
    }

    pub fn take_state(&mut self) -> EvmState {
        std::mem::take(&mut self.result_and_state.state)
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
