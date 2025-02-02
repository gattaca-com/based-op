#![allow(unused)] // TODO: remove

use std::{
    fmt::Display,
    sync::Arc,
    time::{Duration, Instant},
};

use alloy_consensus::Block;
use alloy_rpc_types::engine::{ExecutionPayload, ExecutionPayloadSidecar, ExecutionPayloadV3, PayloadError};
use bop_common::{
    communication::{
        messages::{BlockSyncError, BlockSyncMessage, EngineApi},
        SendersSpine,
    },
    db::{BopDB, BopDbRead},
};
use crossbeam_channel::{Receiver, Sender};
use fetch_blocks::async_fetch_blocks_and_send_sequentially;
use op_alloy_consensus::OpTxEnvelope;
use reqwest::{Client, Url};
use reth_consensus::ConsensusError;
use reth_evm::execute::{
    BlockExecutionError, BlockExecutionOutput, BlockExecutionStrategy, BlockExecutionStrategyFactory, ExecuteOutput,
    InternalBlockExecutionError, ProviderError,
};
use reth_optimism_chainspec::OpChainSpec;
use reth_optimism_evm::OpExecutionStrategyFactory;
use reth_optimism_primitives::{OpBlock, OpReceipt, OpTransactionSigned};
use reth_primitives::{BlockWithSenders, GotExpected};
use reth_primitives_traits::SignedTransaction;
use revm::{db::DbAccount, Database, DatabaseRef};
use tokio::runtime::Runtime;


pub(crate) mod fetch_blocks;

fn payload_to_block(payload: ExecutionPayload, sidecar: ExecutionPayloadSidecar) -> BlockSyncMessage {
    let block = payload.try_into_block_with_sidecar::<OpTransactionSigned>(&sidecar)?;
    let block_senders = block
        .body
        .transactions
        .iter()
        .map(|tx| tx.recover_signer_unchecked())
        .collect::<Option<Vec<_>>>()
        .ok_or(BlockSyncError::SignerRecovery)?;
    Ok(BlockWithSenders { block, senders: block_senders })
}

#[derive(Debug, Clone)]
pub struct BlockSync {
    chain_spec: Arc<OpChainSpec>,
    execution_factory: OpExecutionStrategyFactory,
    runtime: Arc<Runtime>,

    /// Used to fetch blocks from an EL node.
    rpc_url: Url,
}

impl BlockSync {
    /// Creates a new BlockSync instance with the given chain specification and RPC endpoint
    pub fn new(chain_spec: Arc<OpChainSpec>, runtime: Arc<Runtime>, rpc_url: Url) -> Self {
        let execution_factory = OpExecutionStrategyFactory::optimism(chain_spec.clone());
        Self { chain_spec, execution_factory, runtime, rpc_url }
    }

    /// Processes a new execution payload from the engine API.
    /// Commits changes to the database.
    ///
    /// Fetches blocks from the RPC if the sequencer is behind the chain head.
    pub fn apply_new_payload<DB, DbRead>(
        &mut self,
        payload: ExecutionPayload,
        sidecar: ExecutionPayloadSidecar,
        db: DB,
        senders: &SendersSpine<DbRead>,
    ) -> Result<(), BlockSyncError>
    where
        DB: BopDB + BopDbRead + Database<Error: Into<ProviderError> + Display>,
        DbRead: BopDbRead,
    {
        let start = Instant::now();

        let payload_block_number = payload.block_number();
        let cur_block = payload_to_block(payload, sidecar);
        todo!();
        let db_block_head = 0; /* db.block_number()? */
        tracing::info!("handling new payload for block number: {payload_block_number}, db_block_head: {db_block_head}");

        // This case occurs when the sequencer is behind the chain head.
        // This will always happen when the sequencer starts up.
        // We fetch the blocks from the RPC and apply them sequentially.
        if payload_block_number > db_block_head + 1 {
            tracing::info!(
                start_block = db_block_head + 1,
                end_block = payload_block_number - 1,
                "sequencer is behind, fetching blocks"
            );

            self.runtime.spawn(async_fetch_blocks_and_send_sequentially(
                db_block_head + 1,
                payload_block_number - 1,
                self.rpc_url.clone(),
                senders.into(),
                Some(cur_block),
            ));
        } else {
            // Apply and commit the payload block.
            self.apply_and_commit_block(&cur_block?, db)?;
        }
        Ok(())
    }

    /// Executes and validates a block at the current state, committing changes to the database.
    /// Handles chain reorgs by rewinding state if parent hash mismatch is detected.
    pub fn apply_and_commit_block<DB>(&mut self, block: &BlockWithSenders<OpBlock>, db: DB) -> Result<(), BlockSyncError>
    where
        DB: BopDB + BopDbRead + Database<Error: Into<ProviderError> + Display>,
    {
        todo!();
        //debug_assert!(block.header.number == db.block_number()? + 1, "can only apply blocks sequentially");

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

        let execution_output = self.execute(block, db)?;

        // TODO: commit changes here

        Ok(())
    }

    /// Executes a block and validates its state root and receipts.
    /// Returns the execution output containing state changes, receipts, and gas usage.
    pub fn execute<DB>(
        &mut self,
        block: &BlockWithSenders<OpBlock>,
        db: DB,
    ) -> Result<BlockExecutionOutput<OpReceipt>, BlockExecutionError>
    where
        DB: BopDbRead + Database<Error: Into<ProviderError> + Display>,
    {
        let mut start = Instant::now();

        // Apply the block.
        let mut executor = self.execution_factory.create_strategy(db.clone());
        executor.apply_pre_execution_changes(block)?;
        let ExecuteOutput { receipts, gas_used } = executor.execute_transactions(block)?;
        let requests = executor.apply_post_execution_changes(block, &receipts)?;
        let after_block_apply = Instant::now();

        // Validate receipts/ gas used
        reth_optimism_consensus::validate_block_post_execution(block, &self.chain_spec, &receipts)?;

        // Merge transitions and take bundle state.
        let state = executor.finish();

        // Validate state root
        let (state_root, _) = db
            .calculate_state_root(&state)
            .map_err(|e| BlockExecutionError::Internal(InternalBlockExecutionError::Other(e.into())))?;
        if state_root != block.header.state_root {
            return Err(BlockExecutionError::Consensus(ConsensusError::BodyStateRootDiff(
                GotExpected::new(state_root, block.header.state_root).into(),
            )));
        }

        tracing::info!(
            block_number = %block.header.number,
            parent_hash = ?block.header.parent_hash,
            state_root = ?state_root,
            total_latency = ?start.elapsed(),
            block_apply_latency = ?after_block_apply.duration_since(start),
            validation_and_finish_latency = ?after_block_apply.elapsed(),
            "BlockSync::execute finished"
        );

        Ok(BlockExecutionOutput { state, receipts, requests, gas_used })
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use alloy_provider::ProviderBuilder;
    use bop_common::{db::alloy_db::AlloyDB, utils::initialize_test_tracing};
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
        let rpc_url = Url::parse(&rpc_url).unwrap();
        tracing::info!("RPC URL: {}", rpc_url);

        // Create the block executor.
        let chain_spec = Arc::new(OpChainSpecBuilder::base_mainnet().build());
        let mut block_executor = BlockSync::new(chain_spec, rt.clone(), rpc_url.clone());

        // Fetch the block from the RPC.
        let client = Client::builder().timeout(Duration::from_secs(5)).build().expect("Failed to build HTTP client");
        let url = rpc_url.clone();
        let block = rt.block_on(async { fetch_block(25771900, &client, url).await.unwrap() });

        // Create the alloydb.
        let client = ProviderBuilder::new().network().on_http(rpc_url);
        let alloydb = AlloyDB::new(client, block.header.number, rt);

        // Execute the block.
        let res = block_executor.apply_and_commit_block(&block, alloydb);
        tracing::info!("res: {:?}", res);
    }
}
