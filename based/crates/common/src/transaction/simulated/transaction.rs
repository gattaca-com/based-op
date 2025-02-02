use std::{ops::Deref, sync::Arc};

use alloy_primitives::U256;
use revm::DatabaseRef;
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
}
impl SimulatedTx {
    pub fn new<Db>(tx: Arc<Transaction>, result_and_state: ResultAndState, orig_state: Db, coinbase: Address) -> Self
    where
        Db: DatabaseRef,
        <Db as DatabaseRef>::Error: std::fmt::Debug,
    {
        let start_balance = orig_state
            .basic_ref(coinbase)
            .inspect_err(|e| tracing::error!("reading coinbase balance: {e:?}"))
            .ok()
            .flatten()
            .map(|a| a.balance)
            .unwrap_or_default();
        let end_balance = result_and_state.state.get(&coinbase).map(|a| a.info.balance).unwrap_or_default();

        let payment = end_balance.saturating_sub(start_balance);

        Self { tx, result_and_state, payment }
    }

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
