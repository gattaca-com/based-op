use std::sync::Arc;

use bop_common::{db::DBSorting, transaction::SimulatedTx};
use revm::DatabaseCommit;
use alloy_primitives::U256;

/// Fragment of a block being sorted and built
#[derive(Clone, Debug)]
pub struct BuiltFrag<DbRead> {
    pub db: Arc<DBSorting<DbRead>>,
    pub gas_remaining: u64,
    pub payment: U256,
    pub txs: Vec<SimulatedTx>,
    //TODO: bloom receipts etc
}

impl<DbRead: std::fmt::Debug + Clone> BuiltFrag<DbRead> {
    pub fn new(db: DBSorting<DbRead>, max_gas: u64) -> Self {
        Self { db: Arc::new(db), gas_remaining: max_gas, payment: U256::ZERO, txs: vec![] }
    }

    pub fn apply_tx(&mut self, mut tx: SimulatedTx) {
        let db = Arc::make_mut(&mut self.db);
        db.commit(tx.take_state());
        self.payment += tx.payment;
        debug_assert!(
            self.gas_remaining > tx.as_ref().result.gas_used(),
            "had too little gas remaining on block {self:#?} to apply tx {tx:#?}"
        );
        self.gas_remaining -= tx.as_ref().result.gas_used();
        self.txs.push(tx);
    }

    pub fn state(&self) -> Arc<DBSorting<DbRead>> {
        self.db.clone()
    }
}

// 1 add built block, frag state change to frag db, broadcast frag
// State shared across sequencer states
// TODO: add built block
