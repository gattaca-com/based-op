use std::sync::Arc;

use bop_common::transaction::{SimulatedTxList, Transaction};
use bop_db::DBSorting;

pub struct TopOfFragOrders {}

#[derive(Clone, Debug, Default)]
pub struct BuiltBlock<Db> {
    pub state: DBSorting<Db>,
    // CacheDB on top of the current chunk
    pub gas_remaining: u64,
    pub builder_payment: u64,
    pub txs: Vec<Arc<Transaction>>,
    pub tof_snapshot: Vec<SimulatedTxList>,
}
