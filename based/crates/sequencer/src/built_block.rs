use std::sync::Arc;

use bop_common::transaction::{SimulatedTxList, Transaction};

pub struct TopOfFragOrders {}

#[derive(Clone, Debug, Default)]
pub struct BuiltBlock {
    pub gas_remaining: u64,
    pub builder_payment: u64,
    pub txs: Vec<Arc<Transaction>>,
    pub tof_snapshot: Vec<SimulatedTxList>,
    //TODO: bloom receipts etc
}
