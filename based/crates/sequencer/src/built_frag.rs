use std::sync::Arc;

use bop_common::{
    db::DBFrag,
    transaction::{SimulatedTx, SimulatedTxList, Transaction},
};
use revm::{db::CacheDB, DatabaseCommit};
use revm_primitives::U256;

#[derive(Clone, Debug)]
pub struct BuiltFrag<DbRead> {
    pub db: CacheDB<DBFrag<DbRead>>,
    pub gas_remaining: u64,
    pub payment: U256,
    pub txs: Vec<SimulatedTx>,
    //TODO: bloom receipts etc
}

impl<DbRead: std::fmt::Debug> BuiltFrag<DbRead> {
    pub fn new(db: CacheDB<DBFrag<DbRead>>, max_gas: u64) -> Self {
        Self { db, gas_remaining: max_gas, payment: U256::ZERO, txs: vec![] }
    }

    pub fn apply_tx(&mut self, mut tx: SimulatedTx) {
        self.db.commit(tx.take_state());
        self.payment += tx.payment;
        debug_assert!(
            self.gas_remaining > tx.as_ref().result.gas_used(),
            "had too little gas remaining on block {self:#?} to apply tx {tx:#?}"
        );
        self.gas_remaining -= tx.as_ref().result.gas_used();
        self.txs.push(tx);
    }
}
