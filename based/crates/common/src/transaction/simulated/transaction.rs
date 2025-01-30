use std::{ops::Deref, sync::Arc};

use alloy_primitives::{B256, U256};
use revm_primitives::ResultAndState;

use crate::transaction::Transaction;

#[derive(Clone, Debug)]
pub struct SimulatedTx {
    /// original tx
    pub tx: Arc<Transaction>,
    /// revm execution result. Contains gas_used, logs, output, etc.
    pub result_and_state: Arc<ResultAndState>,
    /// Coinbase balance diff, after_sim - before_sim
    pub net_payment: U256,
    /// Parent hash the tx was simulated at
    pub simulated_at_parent_hash: B256,
}

impl Deref for SimulatedTx {
    type Target = Arc<Transaction>;

    fn deref(&self) -> &Self::Target {
        &self.tx
    }
}
