use std::sync::Arc;

use alloy_primitives::U256;
use bop_common::{
    db::{DBFrag, DBSorting},
    p2p::{Frag, FragMessage},
    transaction::SimulatedTx,
};
use bop_db::BopDbRead;
use op_alloy_rpc_types_engine::OpExecutionPayloadEnvelopeV3;
use revm_primitives::B256;

/// Sequence of frags applied on the last block
#[derive(Clone, Debug)]
pub struct FragSequence<Db> {
    db: DBFrag<Db>,
    gas_remaining: u64,
    payment: U256,
    txs: Vec<SimulatedTx>,
    /// Next frag index
    next_seq: u64,
    /// Block number for all frags in this block
    block_number: u64,
}

impl<Db: BopDbRead + Clone + std::fmt::Debug> FragSequence<Db> {
    pub fn new(db: DBFrag<Db>, max_gas: u64) -> Self {
        let block_number = db.block_number().expect("can't get block number") + 1;
        Self { db, gas_remaining: max_gas, payment: U256::ZERO, txs: vec![], next_seq: 0, block_number }
    }

    pub fn db(&self) -> DBFrag<Db> {
        self.db.clone()
    }

    pub fn db_ref(&self) -> &DBFrag<Db> {
        &self.db
    }

    /// Builds a new in-sort frag
    pub fn create_in_sort(&self) -> InSortFrag<Db> {
        let db_sort = DBSorting::new(self.db());
        InSortFrag::new(db_sort, self.gas_remaining)
    }

    /// Creates a new frag, all subsequent frags will be built on top of this one
    pub fn apply_sorted_frag(&mut self, in_sort: InSortFrag<Db>) -> FragMessage {
        self.gas_remaining -= in_sort.gas_used;
        self.payment += in_sort.payment;

        let msg = FragMessage::Frag(Frag {
            block_number: self.block_number,
            seq: self.next_seq,
            txs: in_sort.txs.iter().map(|tx| tx.tx.encode()).collect(),
        });

        self.db.commit(in_sort.db);
        self.txs.extend(in_sort.txs);
        self.next_seq += 1;

        msg
    }

    /// When a new block is received, we clear all the temp state on the db
    pub fn clear_frags(&mut self) {
        self.db.clear();
    }

    pub fn seal_block(&self) -> (FragMessage, OpExecutionPayloadEnvelopeV3) {
        // clear all
        todo!()
    }

    pub fn is_valid(&self, unique_hash: B256) -> bool {
        unique_hash == self.db.unique_hash
    }
}

/// Fragment of a block being sorted and built
#[derive(Clone, Debug)]
pub struct InSortFrag<Db> {
    pub db: Arc<DBSorting<Db>>,
    pub gas_remaining: u64,
    pub gas_used: u64,
    pub payment: U256,
    pub txs: Vec<SimulatedTx>,
}

impl<Db: std::fmt::Debug + Clone> InSortFrag<Db> {
    pub fn new(db: DBSorting<Db>, max_gas: u64) -> Self {
        Self { db: Arc::new(db), gas_remaining: max_gas, gas_used: 0, payment: U256::ZERO, txs: vec![] }
    }

    pub fn apply_tx(&mut self, mut tx: SimulatedTx) {
        let db = Arc::make_mut(&mut self.db);
        db.commit(tx.take_state());
        self.payment += tx.payment;

        // TODO: check gas usage
        let gas_used = tx.as_ref().result.gas_used();
        debug_assert!(
            self.gas_remaining > gas_used,
            "had too little gas remaining on block {self:#?} to apply tx {tx:#?}"
        );

        self.gas_remaining -= gas_used;
        self.gas_used += gas_used;
        self.txs.push(tx);
    }

    pub fn state(&self) -> Arc<DBSorting<Db>> {
        self.db.clone()
    }
}
