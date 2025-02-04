use std::{
    fmt::{Debug, Formatter},
    sync::Arc, time::Instant,
};

use parking_lot::RwLock;
use reth_db::{tables, transaction::DbTxMut, DatabaseEnv};
use reth_node_types::NodeTypesWithDBAdapter;
use reth_optimism_node::OpNode;
use reth_optimism_primitives::{OpBlock, OpReceipt};
use reth_primitives::{BlockWithSenders, Receipts};
use reth_provider::{
    BlockExecutionOutput, ExecutionOutcome, LatestStateProviderRef, ProviderFactory, StateWriter, TrieWriter,
};
use reth_storage_api::{HashedPostStateProvider, StorageLocation};
use reth_trie_common::updates::TrieUpdates;
use revm::{
    db::{BundleState, OriginalValuesKnown},
    Database, DatabaseRef,
};
use revm_primitives::{db::DatabaseCommit, Account, AccountInfo, Address, Bytecode, HashMap, B256, U256};

pub mod alloy_db;
mod block;
mod cache;
mod init;
mod util;

pub use bop_common::db::{BopDB, BopDbRead, Error};
pub use init::init_database;

use crate::{block::BlockDB, cache::ReadCaches};

pub struct DB {
    pub factory: ProviderFactory<NodeTypesWithDBAdapter<OpNode, Arc<DatabaseEnv>>>,
    caches: ReadCaches,
    block: RwLock<Option<BlockDB>>,
}

impl DB {
    pub fn state_root(&self) -> Result<B256, Error> {
        self.readonly()?.state_root()
    }
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

impl DatabaseRef for DB {
    type Error = <<DB as BopDB>::ReadOnly as DatabaseRef>::Error;

    fn basic_ref(&self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        self.readonly()?.basic(address)
    }

    fn code_by_hash_ref(&self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        self.readonly()?.code_by_hash(code_hash)
    }

    fn storage_ref(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
        self.readonly()?.storage(address, index)
    }

    fn block_hash_ref(&self, number: u64) -> Result<B256, Self::Error> {
        self.readonly()?.block_hash(number)
    }
}

impl Database for DB {
    #[doc = " The database error type."]
    type Error = bop_common::db::Error;

    #[doc = " Get basic account information."]
    fn basic(&mut self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        self.readonly()?.basic(address)
    }

    #[doc = " Get account code by its hash."]
    fn code_by_hash(&mut self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        self.readonly()?.code_by_hash(code_hash)
    }

    #[doc = " Get storage value of address at index."]
    fn storage(&mut self, address: Address, index: U256) -> Result<U256, Self::Error> {
        self.readonly()?.storage(address, index)
    }

    #[doc = " Get block hash by block number."]
    fn block_hash(&mut self, number: u64) -> Result<B256, Self::Error> {
        self.readonly()?.block_hash(number)
    }
}

impl BopDbRead for DB {
    fn get_nonce(&self, address: Address) -> u64 {
        self.readonly().map(|db| db.get_nonce(address)).unwrap_or_default()
    }

    fn calculate_state_root(&self, bundle_state: &BundleState) -> Result<(B256, TrieUpdates), Error> {
        self.readonly()?.calculate_state_root(bundle_state)
    }

    fn unique_hash(&self) -> B256 {
        self.readonly().unwrap().unique_hash() // TODO: handle error
    }

    fn block_number(&self) -> Result<u64, Error> {
        self.readonly()?.block_number()
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

        self.commit_block_unchecked(block, block_execution_output, trie_updates)
    }

    /// Commit a new block to the database without performing state root check. This should only be
    /// used if the state root calculation has already been performed upstream.
    fn commit_block_unchecked(
        &self,
        block: &BlockWithSenders<OpBlock>,
        block_execution_output: BlockExecutionOutput<OpReceipt>,
        trie_updates: TrieUpdates,
    ) -> Result<(), Error> {
        let provider = self.factory.provider().map_err(Error::ProviderError)?;
        let rw_provider = self.factory.provider_rw().map_err(Error::ProviderError)?;

        let start = Instant::now();

        // Update the read caches.
        self.caches.update(&block_execution_output.state);
        let after_caches_update = Instant::now();

        let (plain_state, reverts) = block_execution_output.state.to_plain_state_and_reverts(OriginalValuesKnown::Yes);

        // Write state reverts
        rw_provider.write_state_reverts(reverts, block.block.header.number)?;
        let after_state_reverts = Instant::now();
        // Write plain state
        rw_provider.write_state_changes(plain_state)?;
        let after_state_changes = Instant::now();

        // Write state trie updates
        let latest_state = LatestStateProviderRef::new(&provider);
        let hashed_state = latest_state.hashed_post_state(&block_execution_output.state);
        rw_provider.write_hashed_state(&hashed_state.into_sorted()).map_err(Error::ProviderError)?;
        rw_provider.write_trie_updates(&trie_updates).map_err(Error::ProviderError)?;
        let after_trie_updates = Instant::now();

        // Write to header table
        rw_provider
            .tx_ref()
            .put::<tables::CanonicalHeaders>(block.block.header.number, block.block.header.hash_slow())
            .unwrap();

        let after_header_write = Instant::now();

        rw_provider.commit()?;

        let after_commit = Instant::now();

        // Reset the read provider
        *self.block.write() = None;

        tracing::info!(
            "Commit block took: {:?} (caches: {:?}, state_reverts: {:?}, state_changes: {:?}, trie_updates: {:?}, header_write: {:?}, commit: {:?})",
            start.elapsed(),
            after_caches_update.duration_since(start),
            after_state_reverts.duration_since(after_caches_update),
            after_state_changes.duration_since(after_state_reverts),
            after_trie_updates.duration_since(after_state_changes),
            after_header_write.duration_since(after_trie_updates),
            after_commit.duration_since(after_header_write),
        );

        Ok(())
    }
}
