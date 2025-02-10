use std::sync::Arc;

use op_alloy_rpc_types::OpTransactionReceipt;
use parking_lot::RwLock;
use revm_primitives::{HashMap, B256};

use crate::db::DBFrag;

/// Shared state between Sequencer and RPC
/// Allows for access to the State and Receipts
/// Receipts and State are updated when a frag gets sealed
#[derive(Clone, Debug)]
pub struct SharedState<Db> {
    db: DBFrag<Db>,
    receipts: Arc<RwLock<HashMap<B256, OpTransactionReceipt>>>,
}

impl<Db> SharedState<Db> {
    pub fn new(db: DBFrag<Db>) -> Self {
        Self { db, receipts: Arc::new(RwLock::new(Default::default())) }
    }

    /// Get the receipt for a given tx
    pub fn receipt(&self, tx_hash: &B256) -> Option<OpTransactionReceipt> {
        self.receipts.read().get(tx_hash).cloned()
    }

    pub fn insert_receipt(&mut self, tx_hash: B256, receipt: OpTransactionReceipt) {
        self.receipts.write().insert(tx_hash, receipt);
    }
}

impl<Db: Clone> From<&SharedState<Db>> for DBFrag<Db> {
    fn from(value: &SharedState<Db>) -> Self {
        value.db.clone()
    }
}

impl<Db> AsMut<DBFrag<Db>> for SharedState<Db> {
    fn as_mut(&mut self) -> &mut DBFrag<Db> {
        &mut self.db
    }
}

impl<Db> AsRef<DBFrag<Db>> for SharedState<Db> {
    fn as_ref(&self) -> &DBFrag<Db> {
        &self.db
    }
}
