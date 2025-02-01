use std::{ops::Deref, sync::Arc};

use alloy_primitives::{B256, U256};
use revm_primitives::{EvmState, ResultAndState};

use crate::transaction::Transaction;

#[derive(Clone, Debug)]
pub struct SimulatedTx {
    /// original tx
    pub tx: Arc<Transaction>,
    /// revm execution result. Contains gas_used, logs, output, etc.
    pub result_and_state: ResultAndState,
    /// Coinbase balance diff, after_sim - before_sim
    pub payment: u64,
    /// Parent hash the tx was simulated at
    pub simulated_at_parent_hash: B256,
}
impl SimulatedTx {
    pub fn take_state(&mut self) -> EvmState {
        std::mem::take(&mut self.result_and_state.state)
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
