use std::{collections::HashMap, sync::Arc};

use alloy_primitives::{Address, B256, U256};
use revm_primitives::{Account, ExecutionResult};

use crate::transaction::Transaction;

#[derive(Clone, Debug)]
pub struct SimulatedTx {
    /// original tx
    pub tx: Arc<Transaction>,
    /// revm execution result. Contains gas_used, logs, output, etc.
    pub result: ExecutionResult,
    /// revm state changes from the tx.
    pub state_changes: HashMap<Address, Account>,
    /// Coinbase balance diff, after_sim - before_sim
    pub net_payment: U256,
    /// Parent hash the tx was simulated at
    pub simulated_at_parent_hash: B256,
}

impl SimulatedTx {
    pub fn sender(&self) -> Address {
        self.tx.sender()
    }

    pub fn sender_ref(&self) -> &Address {
        self.tx.sender_ref()
    }

    pub fn hash(&self) -> B256 {
        self.tx.hash()
    }
}
