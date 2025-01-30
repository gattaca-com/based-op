use std::ops::Deref;

use crate::transaction::{simulated::transaction::SimulatedTx, TxList};

/// A list of simulated transactions from a single sender.
/// nonce-sorted, i.e. txs[0].nonce = state[address].nonce + 1.
/// First is Simulated Top Of Block
#[derive(Clone, Debug)]
pub struct SimulatedTxList {
    pub first: SimulatedTx,
    pub txs: TxList,
}

impl SimulatedTxList {
    pub fn new(first: SimulatedTx, txs: TxList) -> SimulatedTxList {
        SimulatedTxList { first, txs }
    }

    pub fn len(&self) -> usize {
        self.txs.len() + 1
    }
}

impl Deref for SimulatedTxList {
    type Target = SimulatedTx;

    fn deref(&self) -> &Self::Target {
        &self.first
    }
}
