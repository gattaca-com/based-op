#![allow(unused)] // TODO: remove

use std::{fmt::Display, sync::Arc};

use reth_evm::execute::{
    BlockExecutionError, BlockExecutionOutput, BlockExecutionStrategy, BlockExecutionStrategyFactory, ExecuteOutput,
    ProviderError,
};
use reth_optimism_chainspec::OpChainSpec;
use reth_optimism_evm::OpExecutionStrategyFactory;
use reth_optimism_primitives::{OpBlock, OpReceipt};
use reth_primitives::BlockWithSenders;
use revm::Database;

pub(crate) mod fetch_blocks;

#[derive(Debug, Clone)]
pub struct BlockExecutor {
    chain_spec: Arc<OpChainSpec>,
    execution_factory: OpExecutionStrategyFactory,
}

impl BlockExecutor {
    pub fn new(chain_spec: Arc<OpChainSpec>) -> Self {
        let execution_factory = OpExecutionStrategyFactory::optimism(chain_spec.clone());
        Self { chain_spec, execution_factory }
    }

    /// Execute a block and return the final execution output.
    /// Verifies the block post execution.
    pub fn execute<DB>(
        &mut self,
        block: &BlockWithSenders<OpBlock>,
        db: DB,
    ) -> Result<BlockExecutionOutput<OpReceipt>, BlockExecutionError>
    where
        DB: Database<Error: Into<ProviderError> + Display>,
    {
        tracing::info!(
            "BlockExecutor::execute called for block number: {}, parent hash: {}",
            block.header.number,
            block.header.parent_hash
        );
        let mut executor = self.execution_factory.create_strategy(db);

        // Apply the block.
        executor.apply_pre_execution_changes(block)?;
        let ExecuteOutput { receipts, gas_used } = executor.execute_transactions(block)?;
        let requests = executor.apply_post_execution_changes(block, &receipts)?;

        // Validate the block.
        reth_optimism_consensus::validate_block_post_execution(block, &self.chain_spec, &receipts)?;

        // Merge transitions and take bundle state.
        let state = executor.finish();
        Ok(BlockExecutionOutput { state, receipts, requests, gas_used })
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use alloy_provider::ProviderBuilder;
    use bop_common::utils::initialize_test_tracing;
    use bop_db::alloy_db::AlloyDB;
    use reqwest::Client;
    use reth_optimism_chainspec::OpChainSpecBuilder;

    use super::*;
    use crate::block_sync::fetch_blocks::{fetch_block, TEST_BASE_RPC_URL};

    const ENV_RPC_URL: &str = "BASE_RPC_URL";

    #[test]
    fn test_block_sync_with_alloydb() {
        initialize_test_tracing(tracing::Level::INFO);
        let rt = Arc::new(tokio::runtime::Runtime::new().unwrap());

        // Get RPC URL from environment
        let rpc_url = std::env::var(ENV_RPC_URL).unwrap_or(TEST_BASE_RPC_URL.to_string());
        tracing::info!("RPC URL: {}", rpc_url);

        // Create the block executor.
        let chain_spec = Arc::new(OpChainSpecBuilder::base_mainnet().build());
        let mut block_executor = BlockExecutor::new(chain_spec);

        // Fetch the block from the RPC.
        let client = Client::builder().timeout(Duration::from_secs(5)).build().expect("Failed to build HTTP client");
        let block = rt.block_on(async { fetch_block(25771900, &client, &rpc_url).await.unwrap() });

        // Create the alloydb.
        let client = ProviderBuilder::new().on_http(rpc_url.parse().unwrap());
        let alloydb = AlloyDB::new(client, block.header.number, rt).expect("failed to create alloydb");

        // Execute the block.
        let res = block_executor.execute(&block, alloydb);
        tracing::info!("res: {:?}", res);
    }
}
