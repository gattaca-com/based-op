use std::sync::Arc;

use alloy_primitives::U256;
use bop_common::{db::DBSorting, transaction::SimulatedTx};

/// Fragment of a block being sorted and built
#[derive(Clone, Debug)]
pub struct InSortFrag<Db> {
    pub db: DBSorting<Db>,
    pub gas_remaining: u64,
    pub gas_used: u64,
    pub payment: U256,
    pub txs: Vec<SimulatedTx>,
}

impl<Db> InSortFrag<Db> {
    pub fn new(db: DBSorting<Db>, max_gas: u64) -> Self {
        Self { db, gas_remaining: max_gas, gas_used: 0, payment: U256::ZERO, txs: vec![] }
    }

    pub fn is_empty(&self) -> bool {
        self.txs.is_empty()
    }
}

impl<Db: Clone> InSortFrag<Db> {
    pub fn apply_tx(&mut self, mut tx: SimulatedTx) {
        self.db.commit(tx.take_state());
        self.payment += tx.payment;

        // TODO: check gas usage
        let gas_used = tx.as_ref().result.gas_used();
        debug_assert!(self.gas_remaining > gas_used, "had too little gas remaining to apply tx {tx:#?}");

        self.gas_remaining -= gas_used;
        self.gas_used += gas_used;
        self.txs.push(tx);
    }

    pub fn state(&self) -> DBSorting<Db> {
        self.db.clone()
    }
}
