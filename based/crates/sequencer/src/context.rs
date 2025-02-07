use std::sync::Arc;

use alloy_consensus::Header;
use alloy_rpc_types::engine::ForkchoiceState;
use bop_common::time::Instant;
use bop_db::DatabaseRead;
use bop_pool::transaction::pool::TxPool;
use op_alloy_rpc_types_engine::OpPayloadAttributes;
use reth_optimism_chainspec::OpChainSpec;
use reth_optimism_forks::OpHardforks;
use revm_primitives::{BlockEnv, Bytes, B256};

use crate::{
    block_sync::BlockSync,
    sorting::{ActiveOrders, SortingData},
    FragSequence, SequencerConfig,
};

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct SequencerContext<Db> {
    pub config: SequencerConfig,
    pub db: Db,
    pub tx_pool: TxPool,
    pub block_env: BlockEnv,
    pub frags: FragSequence<Db>,
    pub block_executor: BlockSync,
    pub parent_hash: B256,
    pub parent_header: Header,
    pub fork_choice_state: ForkchoiceState,
    pub payload_attributes: Box<OpPayloadAttributes>,
}

impl<Db: DatabaseRead> SequencerContext<Db> {
    pub fn new_sorting_data(&self) -> SortingData<Db> {
        SortingData {
            frag: self.frags.create_in_sort(),
            until: Instant::now() + self.config.frag_duration,
            in_flight_sims: 0,
            next_to_be_applied: None,
            tof_snapshot: ActiveOrders::new(self.tx_pool.clone_active()),
        }
    }

    pub fn reset_fragdb(&mut self) {
        self.frags.reset_fragdb(self.db.clone());
    }

    pub fn chain_spec(&self) -> &Arc<OpChainSpec> {
        self.config.evm_config.chain_spec()
    }

    pub fn extra_data(&self) -> Bytes {
        let timestamp = self.payload_attributes.payload_attributes.timestamp;
        if self.chain_spec().is_holocene_active_at_timestamp(timestamp) {
            self.payload_attributes
                .get_holocene_extra_data(self.chain_spec().base_fee_params_at_timestamp(timestamp))
                .expect("couldn't get extra data")
        } else {
            Bytes::default()
        }
    }
}

impl<Db> AsRef<BlockEnv> for SequencerContext<Db> {
    fn as_ref(&self) -> &BlockEnv {
        &self.block_env
    }
}
