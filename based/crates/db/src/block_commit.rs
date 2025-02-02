use reth_optimism_primitives::{OpBlock, OpReceipt};
use reth_primitives::{BlockWithSenders, Receipts};
use reth_provider::{BlockExecutionOutput, ExecutionOutcome, LatestStateProviderRef, StateWriter, TrieWriter};
use reth_storage_api::{HashedPostStateProvider, StorageLocation};
use revm::db::OriginalValuesKnown;

use crate::{BopDB, BopDbRead, Error, DB};

impl DB {
    /// Commit a new block to the database.
    pub fn commit_block(
        &self,
        block: &BlockWithSenders<OpBlock>,
        block_execution_output: BlockExecutionOutput<OpReceipt>,
    ) -> Result<(), Error> {
        // Calculate state root and get trie updates.
        let db_ro = self.readonly()?;
        let (state_root, trie_updates) = db_ro.calculate_state_root(&block_execution_output.state)?;

        if state_root != block.block.header.state_root {
            tracing::error!("State root mismatch: {state_root}, block: {:?}", block.block.header);
            return Err(Error::StateRootError(block.block.header.number));
        }

        // Hashed state and trie changes
        let provider = self.factory.provider().map_err(Error::ProviderError)?;
        let latest_state = LatestStateProviderRef::new(&provider);
        let hashed_state = latest_state.hashed_post_state(&block_execution_output.state);

        let rw_provider = self.factory.provider_rw().map_err(Error::ProviderError)?;

        // Write state and reverts.
        rw_provider
            .write_state(
                ExecutionOutcome {
                    bundle: block_execution_output.state,
                    receipts: Receipts::from(block_execution_output.receipts),
                    first_block: block.block.header.number,
                    requests: vec![block_execution_output.requests],
                },
                OriginalValuesKnown::Yes,
                StorageLocation::Both,
            )
            .map_err(Error::ProviderError)?;

        rw_provider.write_hashed_state(&hashed_state.into_sorted()).map_err(Error::ProviderError)?;
        rw_provider.write_trie_updates(&trie_updates).map_err(Error::ProviderError)?;

        Ok(())
    }
}
