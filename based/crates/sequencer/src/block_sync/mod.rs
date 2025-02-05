// #![allow(unused)] // TODO: remove

use std::{fmt::Display, sync::Arc, time::Instant};

use bop_common::{
    communication::messages::BlockSyncError,
    db::{DatabaseRead, DatabaseWrite},
};
use reth_consensus::ConsensusError;
use reth_evm::execute::{
    BlockExecutionError, BlockExecutionOutput, BlockExecutionStrategy, BlockExecutionStrategyFactory, ExecuteOutput,
    InternalBlockExecutionError, ProviderError,
};
use reth_optimism_chainspec::OpChainSpec;
use reth_optimism_evm::OpExecutionStrategyFactory;
use reth_optimism_primitives::{OpBlock, OpReceipt};
use reth_primitives::{BlockWithSenders, GotExpected};
use reth_trie_common::updates::TrieUpdates;
use revm::Database;

pub mod block_fetcher;
pub mod fetch_blocks;
pub mod mock_fetcher;

#[derive(Debug, Clone)]
pub struct BlockSync {
    chain_spec: Arc<OpChainSpec>,
    execution_factory: OpExecutionStrategyFactory,
}

impl BlockSync {
    /// Creates a new BlockSync instance with the given chain specification and RPC endpoint
    pub fn new(chain_spec: Arc<OpChainSpec>) -> Self {
        let execution_factory = OpExecutionStrategyFactory::optimism(chain_spec.clone());
        Self { chain_spec, execution_factory }
    }

    /// Executes and validates a block at the current state, committing changes to the database.
    /// Handles chain reorgs by rewinding state if parent hash mismatch is detected.
    pub fn apply_and_commit_block<DB>(
        &mut self,
        block: &BlockWithSenders<OpBlock>,
        db: &DB,
        commit_block: bool,
    ) -> Result<(), BlockSyncError>
    where
        DB: DatabaseWrite + DatabaseRead,
    {
        tracing::info!("Applying and committing block: {:?}", block.header.number);
        debug_assert!(block.header.number == db.head_block_number()? + 1, "can only apply blocks sequentially");

        // Reorg check
        if let Ok(db_parent_hash) = db.block_hash_ref(block.header.number.saturating_sub(1)) {
            if db_parent_hash != block.header.parent_hash {
                tracing::warn!(
                    "reorg detected at: {}. db_parent_hash: {db_parent_hash:?}, block_hash: {:?}",
                    block.header.number,
                    block.header.hash_slow()
                );

                // TODO: re-wind the state to the last known good state and sync
                panic!("reorg should be impossible on L2");
            }
        }

        let (execution_output, trie_updates) = self.execute(block, db)?;
        if commit_block {
            db.commit_block_unchecked(block, execution_output, trie_updates)?;
        }

        Ok(())
    }

    /// Executes a block and validates its state root and receipts.
    /// Returns the execution output containing state changes, receipts, and gas usage.
    pub fn execute<DB>(
        &mut self,
        block: &BlockWithSenders<OpBlock>,
        db: &DB,
    ) -> Result<(BlockExecutionOutput<OpReceipt>, TrieUpdates), BlockExecutionError>
    where
        DB: DatabaseRead + Database<Error: Into<ProviderError> + Display>,
    {
        let start = Instant::now();

        // Apply the block.
        let mut executor = self.execution_factory.create_strategy(db.clone());
        executor.apply_pre_execution_changes(block)?;
        let ExecuteOutput { receipts, gas_used } = executor.execute_transactions(block)?;
        let requests = executor.apply_post_execution_changes(block, &receipts)?;
        let after_block_apply = Instant::now();

        // Validate receipts/ gas used
        reth_optimism_consensus::validate_block_post_execution(block, &self.chain_spec, &receipts)?;
        let after_light_validation = Instant::now();

        // Merge transitions and take bundle state.
        let state = executor.finish();
        let after_bundle_state_finish = Instant::now();

        // Validate state root
        let (state_root, trie_updates) = db
            .calculate_state_root(&state)
            .map_err(|e| BlockExecutionError::Internal(InternalBlockExecutionError::Other(e.into())))?;
        if state_root != block.header.state_root {
            return Err(BlockExecutionError::Consensus(ConsensusError::BodyStateRootDiff(
                GotExpected::new(state_root, block.header.state_root).into(),
            )));
        }
        let after_state_root = Instant::now();

        tracing::info!(
            block_number = %block.header.number,
            parent_hash = ?block.header.parent_hash,
            state_root = ?state_root,
            total_latency = ?start.elapsed(),
            block_apply_latency = ?after_block_apply.duration_since(start),
            light_validation_latency = ?after_light_validation.duration_since(after_block_apply),
            bundle_state_finish_latency = ?after_bundle_state_finish.duration_since(after_light_validation),
            state_root_latency = ?after_state_root.duration_since(after_bundle_state_finish),
            "BlockSync::execute finished"
        );

        Ok((BlockExecutionOutput { state, receipts, requests, gas_used }, trie_updates))
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use alloy_consensus::TxReceipt;
    use alloy_provider::ProviderBuilder;
    use bop_common::utils::initialize_test_tracing;
    use bop_db::{alloy_db::AlloyDB, init_database};
    use reqwest::{Client, Url};
    use reth_db::{tables, transaction::DbTx};
    use reth_optimism_chainspec::{OpChainSpecBuilder, BASE_SEPOLIA};
    use revm_primitives::address;
    use tracing::level_filters::LevelFilter;

    use super::*;
    use crate::block_sync::fetch_blocks::{fetch_block, TEST_BASE_RPC_URL, TEST_BASE_SEPOLIA_RPC_URL};

    const ENV_RPC_URL: &str = "BASE_RPC_URL";

    #[test]
    fn test_block_sync_with_alloydb() {
        let rt = Arc::new(tokio::runtime::Runtime::new().unwrap());

        // Get RPC URL from environment
        let rpc_url = std::env::var(ENV_RPC_URL).unwrap_or(TEST_BASE_RPC_URL.to_string());
        let rpc_url = Url::parse(&rpc_url).unwrap();
        tracing::info!("RPC URL: {}", rpc_url);

        // Create the block executor.
        let chain_spec = Arc::new(OpChainSpecBuilder::base_sepolia().build());
        let mut block_sync = BlockSync::new(chain_spec);

        // Fetch the block from the RPC.
        let client = Client::builder().timeout(Duration::from_secs(5)).build().expect("Failed to build HTTP client");
        let url = rpc_url.clone();
        let block = rt.block_on(async { fetch_block(25771900, &client, url).await.unwrap() });

        // Create the alloydb.
        let client = ProviderBuilder::new().network().on_http(rpc_url);
        let alloydb = AlloyDB::new(client, block.header.number, rt);

        // Execute the block.
        let res = block_sync.apply_and_commit_block(&block, &alloydb, false);
        tracing::info!("res: {:?}", res);
    }

    #[test]
    fn test_block_sync_with_on_disk_db() {
        initialize_test_tracing(LevelFilter::INFO);

        // Initialise the on disk db.
        let db_location = std::env::var("DB_LOCATION").unwrap_or_else(|_| "/tmp/base_sepolia".to_string());
        let db: bop_db::SequencerDB = init_database(&db_location, 1000, 1000).unwrap();
        let db_head_block_number = db.head_block_number().unwrap();
        println!("DB Head Block Number: {:?}", db_head_block_number);

        // initialise block sync and fetch block
        let rt = Arc::new(tokio::runtime::Runtime::new().unwrap());
        let rpc_url = Url::parse(TEST_BASE_SEPOLIA_RPC_URL).unwrap();

        // Create the block executor.
        let chain_spec = BASE_SEPOLIA.clone();
        let mut block_sync = BlockSync::new(chain_spec, RuntimeOrHandle::new_runtime(rt.clone()), rpc_url.clone());

        let client = Client::builder().timeout(Duration::from_secs(5)).build().expect("Failed to build HTTP client");
        let block = rt.block_on(async { fetch_block(db_head_block_number + 1, &client, rpc_url).await.unwrap() });

        // Execute the block.
        assert!(block_sync.execute(&block, &db).is_ok());
    }
}
