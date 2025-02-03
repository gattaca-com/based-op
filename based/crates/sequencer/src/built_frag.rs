use std::sync::Arc;

use alloy_primitives::U256;
use bop_common::{db::DBSorting, transaction::SimulatedTx};

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

    pub fn apply_tx(mut self, mut tx: SimulatedTx) -> Self {
        let mut db = Arc::unwrap_or_clone(self.db);
        db.commit(tx.take_state());
        self.db = Arc::new(db);
        self.payment += tx.payment;
        debug_assert!(
            self.gas_remaining > tx.as_ref().result.gas_used(),
            "had too little gas remaining on block {self:#?} to apply tx {tx:#?}"
        );
        self.gas_remaining -= tx.as_ref().result.gas_used();
        self.txs.push(tx);
        self
    }

    pub fn state(&self) -> Arc<DBSorting<DbRead>> {
        self.db.clone()
    }
}

// 1 add built block, frag state change to frag db, broadcast frag
// State shared across sequencer states
// TODO: add built block
