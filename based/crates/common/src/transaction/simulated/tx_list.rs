use std::sync::Arc;

use alloy_primitives::Address;

use crate::transaction::simulated::transaction::SimulatedTx;

/// A list of simulated transactions from a single sender.
/// nonce-sorted and all can be applied from the top-of-block state
/// i.e., txs[0].nonce = state[address].nonce + 1.
#[derive(Clone, Debug, Default)]
pub struct SimulatedTxList {
    pub txs: Vec<Arc<SimulatedTx>>,
    pub sender: Address,
}

impl SimulatedTxList {
    pub fn new(txs: Vec<Arc<SimulatedTx>>) -> SimulatedTxList {
        SimulatedTxList { sender: txs[0].sender(), txs }
    }

    pub fn len(&self) -> usize {
        self.txs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.txs.is_empty()
    }
}
