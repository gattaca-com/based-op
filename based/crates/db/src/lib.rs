use std::{
    fmt::{Debug, Formatter},
    sync::Arc,
};

use parking_lot::RwLock;
use reth_db::DatabaseEnv;
use reth_node_types::NodeTypesWithDBAdapter;
use reth_optimism_node::OpNode;
use reth_optimism_primitives::{OpBlock, OpReceipt};
use reth_primitives::{BlockWithSenders, Receipts};
use reth_provider::{
    BlockExecutionOutput, ExecutionOutcome, LatestStateProviderRef, ProviderFactory, StateWriter, TrieWriter,
};
use reth_storage_api::{HashedPostStateProvider, StorageLocation};
use revm::db::OriginalValuesKnown;
use revm_primitives::{db::DatabaseCommit, Account, Address, HashMap};

pub mod alloy_db;
mod block;
mod cache;
mod init;
mod util;

pub use bop_common::db::{BopDB, BopDbRead, Error};
pub use init::init_database;
pub use util::state_changes_to_bundle_state;

use crate::{block::BlockDB, cache::ReadCaches};

pub struct DB {
    factory: ProviderFactory<NodeTypesWithDBAdapter<OpNode, Arc<DatabaseEnv>>>,
    caches: ReadCaches,
    block: RwLock<Option<BlockDB>>,
}

impl Clone for DB {
    fn clone(&self) -> Self {
        Self { factory: self.factory.clone(), caches: self.caches.clone(), block: RwLock::new(None) }
    }
}

impl Debug for DB {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str("DB")
    }
}

impl BopDB for DB {
    type ReadOnly = BlockDB;

    fn readonly(&self) -> Result<Self::ReadOnly, Error> {
        if let Some(block) = self.block.read().as_ref().cloned() {
            return Ok(block);
        }

        let block = BlockDB::new(self.caches.clone(), self.factory.provider().map_err(Error::ProviderError)?);
        self.block.write().replace(block.clone());
        Ok(block)
    }

    /// Commit a new block to the database.
    fn commit_block(
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

impl DatabaseCommit for DB {
    // TODO not the place to commit to DB - cannot return anything or errors here.
    fn commit(&mut self, changes: HashMap<Address, Account>) {
        let ro_db = self.readonly().expect("failed to create ro db");
        let bundle_state =
            util::state_changes_to_bundle_state(&ro_db, changes).expect("failed to convert to bundle state");
        let (_root, _trie_updates) = ro_db.calculate_state_root(&bundle_state).expect("failed to calc state root");

        // TODO write updates

        // Update the read caches.
        self.caches.update(&bundle_state);
    }
}
