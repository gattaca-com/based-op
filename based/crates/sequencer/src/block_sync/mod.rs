// #![allow(unused)] // TODO: remove

use std::{fmt::Display, sync::Arc};

use bop_common::{
    communication::messages::BlockSyncError,
    db::{DatabaseRead, DatabaseWrite},
    time::BlockSyncTimers,
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
use tracing::{info, warn};

pub mod block_fetcher;
pub mod fetch_blocks;
pub mod mock_fetcher;

pub type AlloyProvider =
    alloy_provider::RootProvider<alloy_transport_http::Http<reqwest::Client>, op_alloy_network::Optimism>;

#[derive(Debug, Clone)]
pub struct BlockSync {
    chain_spec: Arc<OpChainSpec>,
    execution_factory: OpExecutionStrategyFactory,
    /// Blocks that we have received from the provider but require a prior block to be applied before this can be.
    /// Sorted list in reverse order by block number.
    pending_blocks: Vec<BlockWithSenders<OpBlock>>,
    timers: BlockSyncTimers,
}

impl BlockSync {
    /// Creates a new BlockSync instance with the given chain specification and RPC endpoint
    pub fn new(chain_spec: Arc<OpChainSpec>) -> Self {
        let execution_factory = OpExecutionStrategyFactory::optimism(chain_spec.clone());
        Self { chain_spec, execution_factory, pending_blocks: vec![], timers: Default::default() }
    }

    /// Returns block numbers to fetch, start to end. This will be used in the case of a reorg.
    #[tracing::instrument(skip_all, fields(block = %block.header.number))]
    pub fn commit_block<DB>(
        &mut self,
        block: &BlockWithSenders<OpBlock>,
        db: &DB,
        commit_block: bool,
    ) -> Result<Option<(u64, u64)>, BlockSyncError>
    where
        DB: DatabaseWrite + DatabaseRead,
    {
        self.timers.total.start();
        let db_head = db.head_block_number()?;
        let block_number = block.header.number;

        // If the block number is greater than the head, we can apply it directly.
        if block_number > db_head + 1 {
            warn!("got a block with a number greater than the head. Block number: {block_number}. Head block number: {db_head}. Inserting into pending blocks.");
            self.insert_pending_block(block);
            return Ok(Some((db_head + 1, block_number - 1)));
        }

        // Check if we committed a different block with the same number and rewind the database if so.
        if block_number <= db_head {
            let new_block_hash = block.header.hash_slow();

            // Check if the block has already been committed.
            let db_block_hash = db.block_hash_ref(block_number).expect("failed to get block hash"); // TODO: handle DatabaseRef errors
            if db_block_hash == new_block_hash {
                return Ok(None);
            }

            // Roll back the database until we reach new_block_number-1.
            warn!("got a block with previously committed number and different hash to the one committed. Rolling back database. Block number: {block_number}. Block hash: {new_block_hash:?}. DB head block number: {block_number}. DB head block hash: {db_block_hash:?}");
            while db.head_block_number()? >= block_number {
                db.roll_back_head()?;
            }
        }

        let db_head_hash = db.head_block_hash().expect("failed to get head block hash");
        if db_head_hash != block.header.parent_hash {
            warn!(
                "reorg detected. new block parent doesn't match db head. Block number: {}. Block parent hash: {:?}, db_head_hash: {:?}",
                block.header.number,
                block.header.parent_hash,
                db_head_hash
            );

            // Roll back head and request missing blocks.
            db.roll_back_head()?;
            self.insert_pending_block(block);

            return Ok(Some((db.head_block_number().unwrap() + 1, block.header.number)));
        }

        self.execute_and_maybe_commit(block, db, commit_block)?;

        // Process any pending blocks that can now be applied
        while let Some(last_pending) = self.pending_blocks.last() {
            // Check if the next block can be applied
            if last_pending.header.number != db.head_block_number()? + 1 {
                break;
            }

            let pending_block = self.pending_blocks.pop().unwrap();

            // Verify block links to current chain head
            if pending_block.header.parent_hash != db.head_block_hash().expect("failed to get head block hash") {
                warn!(
                    "pending block parent hash mismatch. Block number: {}, Expected parent: {:?}, Got: {:?}",
                    pending_block.header.number,
                    db.head_block_hash().expect("failed to get head block hash"),
                    pending_block.header.parent_hash
                );
                debug_assert!(false, "pending block parent hash doesn't match db head hash");

                db.roll_back_head()?;
                // Request to re-fetch rolled back block and the pending block.
                return Ok(Some((pending_block.header.number - 1, pending_block.header.number)));
            }

            self.execute_and_maybe_commit(&pending_block, db, commit_block)?;
        }
        self.timers.total.stop();
        info!(
            n_txs = block.body.transactions.len(),
            total_t = %self.timers.total.elapsed(),
        );
        tracing::debug!(
            "commit block took: {} (caches: {}, state_changes: {}, trie_updates: {}, header_write: {}, db_commit: {})",
            self.timers.total.elapsed(),
            self.timers.caches.elapsed(),
            self.timers.state_changes.elapsed(),
            self.timers.trie_updates.elapsed(),
            self.timers.header_write.elapsed(),
            self.timers.db_commit.elapsed(),
        );

        Ok(None)
    }

    pub fn execute_and_maybe_commit<DB>(
        &mut self,
        block: &BlockWithSenders<OpBlock>,
        db: &DB,
        commit: bool,
    ) -> Result<(), BlockSyncError>
    where
        DB: DatabaseWrite + DatabaseRead + Database<Error: Into<ProviderError> + Display>,
    {
        self.timers.execution.start();
        let (execution_output, trie_updates) = self.execute(block, db)?;
        self.timers.execution.stop();
        if commit {
            self.timers.db_commit.start();
            db.commit_block_unchecked(block, execution_output, trie_updates, &mut self.timers)?;
            self.timers.db_commit.stop();
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
        debug_assert!(
            block.header.parent_hash == db.head_block_hash().expect("failed to get head block hash"),
            "can only apply blocks sequentially"
        );
        self.timers.execute_txs.start();
        // Apply the block.
        let mut executor = self.execution_factory.create_strategy(db.clone());
        executor.apply_pre_execution_changes(block)?;
        let ExecuteOutput { receipts, gas_used } = executor.execute_transactions(block)?;
        let requests = executor.apply_post_execution_changes(block, &receipts)?;
        self.timers.execute_txs.stop();

        // Validate receipts/ gas used
        self.timers
            .validate
            .time(|| reth_optimism_consensus::validate_block_post_execution(block, &self.chain_spec, &receipts))?;

        // Merge transitions and take bundle state.
        let state = self.timers.take_bundle.time(|| executor.finish());

        // Validate state root
        let trie_updates = self.timers.state_root.time(|| {
            let (state_root, trie_updates) = db
                .calculate_state_root(&state)
                .map_err(|e| BlockExecutionError::Internal(InternalBlockExecutionError::Other(e.into())))?;

            if state_root != block.header.state_root {
                return Err(BlockExecutionError::Consensus(ConsensusError::BodyStateRootDiff(
                    GotExpected::new(state_root, block.header.state_root).into(),
                )));
            }
            Ok(trie_updates)
        })?;
        self.timers.state_root.stop();

        Ok((BlockExecutionOutput { state, receipts, requests, gas_used }, trie_updates))
    }

    fn insert_pending_block(&mut self, block: &BlockWithSenders<OpBlock>) {
        let index = self
            .pending_blocks
            .binary_search_by(|pending_block| pending_block.header.number.cmp(&block.header.number).reverse())
            .unwrap_or_else(|i| i);
        self.pending_blocks.insert(index, block.clone());
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use alloy_primitives::B256;
    use alloy_provider::ProviderBuilder;
    use bop_common::utils::initialize_test_tracing;
    use bop_db::{init_database, AlloyDB};
    use reqwest::Url;
    use reth_optimism_chainspec::{OpChainSpecBuilder, BASE_SEPOLIA};
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
        info!("RPC URL: {}", rpc_url);

        // Create the block executor.
        let chain_spec = Arc::new(OpChainSpecBuilder::base_sepolia().build());
        let mut block_sync = BlockSync::new(chain_spec);

        // Fetch the block from the RPC.
        let provider = ProviderBuilder::new().network().on_http(rpc_url);
        let block = rt.block_on(async { fetch_block(25771900, &provider).await });

        // Create the alloydb.
        let alloydb = AlloyDB::new(provider, block.header.number, rt);

        // Execute the block.
        let res = block_sync.commit_block(&block, &alloydb, false);
        info!("res: {:?}", res);
    }

    #[test]
    fn test_block_sync_with_on_disk_db() {
        initialize_test_tracing(LevelFilter::INFO);

        // Initialise the on disk db.
        let db_location = std::env::var("DB_LOCATION")
            .unwrap_or_else(|_| "/Users/georgedavies/gattaca-com/rust/based-op/based/base_sepolia".to_string());
        let db: bop_db::SequencerDB = init_database(&db_location, 1000, 1000, BASE_SEPOLIA.clone()).unwrap();
        let db_head_block_number = db.head_block_number().unwrap();
        println!("DB Head Block Number: {:?}", db_head_block_number);

        // initialise block sync and fetch block
        let rt = Arc::new(tokio::runtime::Runtime::new().unwrap());
        let rpc_url = Url::parse(TEST_BASE_SEPOLIA_RPC_URL).unwrap();

        // Create the block executor.
        let chain_spec = BASE_SEPOLIA.clone();
        let mut block_sync = BlockSync::new(chain_spec);

        let provider = ProviderBuilder::new().network().on_http(rpc_url);
        let block = rt.block_on(async { fetch_block(db_head_block_number + 1, &provider).await });

        // Execute the block.
        assert!(block_sync.commit_block(&block, &db, true).is_ok());
    }

    #[test]
    fn test_block_sync_reorgs() {
        initialize_test_tracing(LevelFilter::INFO);

        // Setup test environment
        let db_location = std::env::var("DB_LOCATION")
            .unwrap_or_else(|_| "/Users/georgedavies/gattaca-com/rust/based-op/based/base_sepolia".to_string());
        let db: bop_db::SequencerDB = init_database(&db_location, 1000, 1000, BASE_SEPOLIA.clone()).unwrap();
        let rt = Arc::new(tokio::runtime::Runtime::new().unwrap());
        let rpc_url = Url::parse(TEST_BASE_SEPOLIA_RPC_URL).unwrap();
        let provider = ProviderBuilder::new().network().on_http(rpc_url);

        let mut block_sync = BlockSync::new(BASE_SEPOLIA.clone());

        // Get initial block number from db
        let start_block = db.head_block_number().unwrap();

        // Fetch a sequence of blocks for testing
        let mut blocks = HashMap::new();
        for i in 0..5 {
            let block = rt.block_on(async { fetch_block(start_block - 1 + i, &provider).await });
            blocks.insert(block.header.number, block);
        }

        for block in blocks.values() {
            tracing::info!("Header: {:?}", block.header);
        }

        let state_root = db.state_root().unwrap();
        let block = blocks.get(&start_block).unwrap();
        tracing::info!(
            "State Root: {:?}, Block Number: {:?}, Block State Root: {:?}",
            state_root,
            start_block,
            block.header.state_root
        );

        // Test Case 1: reorg at depth 2
        {
            // Apply first block normally
            let block = blocks.get(&(start_block + 1)).unwrap();
            let result = block_sync.commit_block(block, &db, true);
            tracing::info!("Result: {:?}", result);
            assert!(result.is_ok());
            assert!(result.unwrap().is_none());

            // Create a competing block at same height
            let mut competing_block = block.clone();
            competing_block.header.parent_hash = B256::random(); // Force different parent hash - we don't commit the header to the db so this won't affect the db.

            // Apply competing block - should trigger reorg but won't ask for new blocks as the height is the same.
            let result = block_sync.commit_block(&competing_block, &db, true);
            assert!(result.is_ok());
            let fetch_request = result.unwrap().expect("should request reorg blocks");

            // Verify correct blocks requested
            match fetch_request {
                BlockFetch::FromTo(from, to) => {
                    assert_eq!(from, competing_block.header.number - 1);
                    assert_eq!(to, competing_block.header.number);
                }
            }

            // Verify db state after reorg has gone past db.
            assert_eq!(db.head_block_number().unwrap(), start_block - 1);
        }

        let head_block = db.head_block_number().unwrap();
        let state_root = db.state_root().unwrap();
        tracing::info!("Head Block Number after first reorg: {:?} | State Root: {:?}", head_block, state_root);

        // Test Case 2: Reorg with missing intermediate block
        {
            // Apply blocks from head_block+1 to head_block + 3, skipping head_block + 2
            let block1 = blocks.get(&(head_block + 1)).unwrap();
            tracing::info!("Block 1: {:?}", block1.header.number);
            let result = block_sync.commit_block(block1, &db, true);
            tracing::info!("Result: {:?}", result);
            assert!(result.is_ok());

            let block3 = blocks.get(&(head_block + 3)).unwrap();
            let result = block_sync.commit_block(block3, &db, true);
            assert!(result.is_ok());
            let fetch_request = result.unwrap().expect("should request missing blocks");

            // Verify correct range requested
            match fetch_request {
                BlockFetch::FromTo(from, to) => {
                    assert_eq!(from, head_block + 2);
                    assert_eq!(to, head_block + 2);
                }
            }

            // Verify block is in pending queue
            assert_eq!(block_sync.pending_blocks.len(), 1);
            assert_eq!(block_sync.pending_blocks[0].header.number, head_block + 3);
        }

        // Test Case 3: Apply pending blocks after gap is filled
        {
            let block2 = blocks.get(&(head_block + 2)).unwrap();
            let result = block_sync.commit_block(block2, &db, true);
            assert!(result.is_ok());
            assert!(result.unwrap().is_none()); // No more blocks needed

            // Verify pending block was processed
            assert_eq!(block_sync.pending_blocks.len(), 0);
            assert_eq!(db.head_block_number().unwrap(), head_block + 3);
        }
    }
}
