use std::{collections::VecDeque, sync::Arc};

use alloy_consensus::Header;
use alloy_rpc_types::engine::ForkchoiceState;
use bop_common::{time::Instant, transaction::Transaction};
use bop_db::DatabaseRead;
use bop_pool::transaction::pool::TxPool;
use op_alloy_rpc_types_engine::OpPayloadAttributes;
use revm_primitives::{BlockEnv, B256};

use crate::{
    block_sync::BlockSync,
    sorting::{ActiveOrders, SortingData},
    FragSequence, SequencerConfig,
};

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct SequencerContext<Db: DatabaseRead> {
    pub config: SequencerConfig,
    pub db: Db,
    pub tx_pool: TxPool,
    pub block_env: BlockEnv,
    pub frags: FragSequence<Db>,
    pub block_executor: BlockSync,
    pub parent_hash: B256,
    pub parent_header: Header,
    pub parent_beacon_block_root: B256,
    pub fork_choice_state: ForkchoiceState,
    pub payload_attributes: Box<OpPayloadAttributes>,
    //TODO: set from blocksync
    pub base_fee: u64,
}

impl<Db: DatabaseRead> SequencerContext<Db> {
    pub fn new_sorting_data(
        &self,
        remaining_attributes_txs: VecDeque<Arc<Transaction>>,
        can_add_txs: bool,
    ) -> SortingData<Db> {
        SortingData {
            frag: self.frags.create_in_sort(),
            until: Instant::now() + self.config.frag_duration,
            in_flight_sims: 0,
            next_to_be_applied: None,
            tof_snapshot: ActiveOrders::new(self.tx_pool.clone_active()),
            remaining_attributes_txs,
            can_add_txs,
        }
    }

    pub fn reset_fragdb(&mut self) {
        self.frags.reset_fragdb(self.db.clone());
    }
}
