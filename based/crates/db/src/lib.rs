use std::{
    fmt::{Debug, Formatter},
    sync::Arc,
};

use bop_common::time::BlockSyncTimers;
use parking_lot::RwLock;
use reth_db::{
    cursor::{DbCursorRO, DbCursorRW, DbDupCursorRO},
    models::BlockNumberAddress,
    tables,
    transaction::{DbTx, DbTxMut},
    Bytecodes, CanonicalHeaders, DatabaseEnv,
};
use reth_node_types::NodeTypesWithDBAdapter;
use reth_optimism_node::OpNode;
use reth_optimism_primitives::{OpBlock, OpReceipt};
use reth_primitives::{BlockWithSenders, StorageEntry};
use reth_provider::{
    providers::ConsistentDbView, BlockExecutionOutput, DatabaseProviderRO, LatestStateProviderRef, ProviderFactory,
    StateWriter, TrieWriter,
};
use reth_storage_api::{DBProvider, HashedPostStateProvider};
use reth_trie::{StateRoot, TrieInput};
use reth_trie_common::updates::TrieUpdates;
use reth_trie_db::{DatabaseHashedCursorFactory, DatabaseTrieCursorFactory};
use reth_trie_parallel::root::ParallelStateRoot;
use revm::{
    db::{BundleState, OriginalValuesKnown},
    Database, DatabaseRef,
};
use revm_primitives::{AccountInfo, Address, Bytecode, B256, U256};

mod alloy_db;
mod cache;
mod init;
pub use alloy_db::AlloyDB;
pub use bop_common::db::{DatabaseRead, DatabaseWrite, Error};
pub use init::init_database;

use crate::cache::ReadCaches;

pub type ProviderReadOnly = DatabaseProviderRO<Arc<DatabaseEnv>, NodeTypesWithDBAdapter<OpNode, Arc<DatabaseEnv>>>;

#[derive(Clone)]
pub struct SequencerDB {
    pub factory: ProviderFactory<NodeTypesWithDBAdapter<OpNode, Arc<DatabaseEnv>>>,
    caches: ReadCaches,
    provider: Arc<RwLock<Option<Arc<ProviderReadOnly>>>>,
}

impl SequencerDB {
    pub fn new(
        factory: ProviderFactory<NodeTypesWithDBAdapter<OpNode, Arc<DatabaseEnv>>>,
        max_cached_accounts: u64,
        max_cached_storages: u64,
    ) -> Self {
        let caches = ReadCaches::new(max_cached_accounts, max_cached_storages);
        Self { factory, caches, provider: Arc::new(RwLock::new(None)) }
    }

    pub fn provider(&self) -> Result<Arc<ProviderReadOnly>, Error> {
        if let Some(provider) = self.provider.read().as_ref().cloned() {
            return Ok(provider);
        }

        let provider = Arc::new(self.factory.provider().map_err(Error::ProviderError)?);
        self.provider.write().replace(provider.clone());
        Ok(provider)
    }

    pub fn state_root(&self) -> Result<B256, Error> {
        let provider = self.provider()?;
        let tx = provider.tx_ref();
        StateRoot::new(DatabaseTrieCursorFactory::new(tx), DatabaseHashedCursorFactory::new(tx))
            .root()
            .map_err(Error::RethStateRootError)
    }

    /// Reset the read db provider. Will be lazily re-created on next access.
    /// Should be called when a new block is committed.
    pub fn reset_provider(&self) {
        *self.provider.write() = None;
    }

    /// Puts a canonical header into the database and commits the changes.
    pub fn write_canonical_header(&self, number: u64, hash: B256) -> Result<(), Error> {
        let rw_provider = self.factory.provider_rw().map_err(Error::ProviderError)?;
        rw_provider.tx_ref().put::<tables::CanonicalHeaders>(number, hash).unwrap();
        rw_provider.commit()?;
        Ok(())
    }
}

impl Debug for SequencerDB {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str("DB")
    }
}

impl DatabaseRead for SequencerDB {
    fn calculate_state_root(&self, bundle_state: &BundleState) -> Result<(B256, TrieUpdates), Error> {
        let provider = self.provider()?;
        let latest_state = LatestStateProviderRef::new(provider.as_ref());
        let hashed_state = latest_state.hashed_post_state(bundle_state);
        let consistent_view = ConsistentDbView::new_unchecked(self.factory.clone())?;
        let parallel_state_root = ParallelStateRoot::new(consistent_view, TrieInput::from_state(hashed_state));
        parallel_state_root.incremental_root_with_updates().map_err(Error::ParallelStateRootError)
    }

    fn head_block_number(&self) -> Result<u64, Error> {
        let provider = self.provider()?;
        provider.tx_ref().cursor_read::<CanonicalHeaders>()?.last()?.map_or(Ok(0), |(num, _)| Ok(num))
    }

    fn head_block_hash(&self) -> Result<B256, Error> {
        let provider = self.provider()?;
        provider.tx_ref().cursor_read::<CanonicalHeaders>()?.last()?.map_or(Ok(B256::ZERO), |(_, hash)| Ok(hash))
    }
}

impl DatabaseWrite for SequencerDB {
    /// Commit a new block to the database without performing state root check. This should only be
    /// used if the state root calculation has already been performed upstream.
    fn commit_block_unchecked(
        &self,
        block: &BlockWithSenders<OpBlock>,
        block_execution_output: BlockExecutionOutput<OpReceipt>,
        trie_updates: TrieUpdates,
        timers: &mut BlockSyncTimers,
    ) -> Result<(), Error> {
        let provider = self.factory.provider().map_err(Error::ProviderError)?;
        let rw_provider = self.factory.provider_rw().map_err(Error::ProviderError)?;

        timers.caches.time(|| self.caches.update(&block_execution_output.state));

        // Update the read caches.

        timers.state_changes.time(|| {
            let (plain_state, reverts) =
                block_execution_output.state.to_plain_state_and_reverts(OriginalValuesKnown::Yes);
            // Write state reverts
            rw_provider.write_state_reverts(reverts, block.block.header.number)?;
            // Write plain state
            rw_provider.write_state_changes(plain_state)
        })?;

        // Write state trie updates
        timers.trie_updates.time(|| {
            let latest_state = LatestStateProviderRef::new(&provider);
            let hashed_state = latest_state.hashed_post_state(&block_execution_output.state);
            rw_provider.write_hashed_state(&hashed_state.into_sorted()).map_err(Error::ProviderError)?;
            rw_provider.write_trie_updates(&trie_updates).map_err(Error::ProviderError)
        })?;
        timers.header_write.time(|| {
            // Write to header table
            rw_provider
                .tx_ref()
                .put::<tables::CanonicalHeaders>(block.block.header.number, block.block.header.hash_slow())
                .unwrap();
        });

        timers.db_commit.time(|| rw_provider.commit())?;

        self.reset_provider();

        Ok(())
    }

    /// Removes the last block of state from the database.
    fn roll_back_head(&self) -> Result<(), Error> {
        let rw_provider = self.factory.provider_rw().map_err(Error::ProviderError)?;

        let head_block_number = self.head_block_number()?;
        let range = head_block_number..=head_block_number;
        let storage_range = BlockNumberAddress::range(range.clone());

        // Trie
        rw_provider.unwind_trie_state_range(range.clone())?;

        // Flat state
        let storage_changeset = rw_provider.take::<tables::StorageChangeSets>(storage_range.clone())?;
        let account_changeset = rw_provider.take::<tables::AccountChangeSets>(range)?;

        let mut plain_accounts_cursor = rw_provider.tx_ref().cursor_write::<tables::PlainAccountState>()?;
        let mut plain_storage_cursor = rw_provider.tx_ref().cursor_dup_write::<tables::PlainStorageState>()?;

        let (state, _) = rw_provider.populate_bundle_state(
            account_changeset,
            storage_changeset,
            &mut plain_accounts_cursor,
            &mut plain_storage_cursor,
        )?;

        // iterate over local plain state remove all account and all storages.
        for (address, (old_account, new_account, storage)) in &state {
            // revert account if needed.
            if old_account != new_account {
                let existing_entry = plain_accounts_cursor.seek_exact(*address)?;
                if let Some(account) = old_account {
                    plain_accounts_cursor.upsert(*address, *account)?;
                } else if existing_entry.is_some() {
                    plain_accounts_cursor.delete_current()?;
                }
            }

            // revert storages
            for (storage_key, (old_storage_value, _new_storage_value)) in storage {
                let storage_entry = StorageEntry { key: *storage_key, value: *old_storage_value };
                // delete previous value
                if plain_storage_cursor
                    .seek_by_key_subkey(*address, *storage_key)?
                    .filter(|s| s.key == *storage_key)
                    .is_some()
                {
                    plain_storage_cursor.delete_current()?
                }

                // insert value if needed
                if !old_storage_value.is_zero() {
                    plain_storage_cursor.upsert(*address, storage_entry)?;
                }
            }
        }

        rw_provider.tx_ref().delete::<tables::CanonicalHeaders>(head_block_number, None).unwrap();

        rw_provider.commit()?;
        self.reset_provider();

        Ok(())
    }
}

impl DatabaseRef for SequencerDB {
    type Error = Error;

    fn basic_ref(&self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        self.caches.account_info(&address, self.provider()?.tx_ref())
    }

    fn code_by_hash_ref(&self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        let code = self.provider()?.tx_ref().get::<Bytecodes>(code_hash).map_err(Error::ReadTransactionError)?;
        Ok(code.unwrap_or_default().0)
    }

    fn storage_ref(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
        self.caches.storage(&(address, index), self.provider()?.tx_ref())
    }

    fn block_hash_ref(&self, number: u64) -> Result<B256, Self::Error> {
        let hash = self.provider()?.tx_ref().get::<CanonicalHeaders>(number).map_err(Error::ReadTransactionError)?;
        let hash = hash.ok_or(Error::BlockNotFound(number))?;
        Ok(hash)
    }
}

impl Database for SequencerDB {
    type Error = Error;

    #[inline]
    fn basic(&mut self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        self.basic_ref(address)
    }

    #[inline]
    fn code_by_hash(&mut self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        self.code_by_hash_ref(code_hash)
    }

    #[inline]
    fn storage(&mut self, address: Address, index: U256) -> Result<U256, Self::Error> {
        self.storage_ref(address, index)
    }

    #[inline]
    fn block_hash(&mut self, number: u64) -> Result<B256, Self::Error> {
        self.block_hash_ref(number)
    }
}
