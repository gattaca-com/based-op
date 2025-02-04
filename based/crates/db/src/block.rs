use std::{
    fmt::{Debug, Formatter},
    sync::Arc,
};

use reth_db::{cursor::DbCursorRO, Bytecodes, CanonicalHeaders, DatabaseEnv};
use reth_db_api::transaction::DbTx;
use reth_node_types::NodeTypesWithDBAdapter;
use reth_optimism_node::OpNode;
use reth_provider::{DatabaseProviderRO, LatestStateProviderRef};
use reth_storage_api::{HashedPostStateProvider, StateRootProvider};
use reth_trie::StateRoot;
use reth_trie_common::updates::TrieUpdates;
use reth_trie_db::{DatabaseHashedCursorFactory, DatabaseTrieCursorFactory};
use revm::db::BundleState;
use revm_primitives::{
    db::{Database, DatabaseRef},
    AccountInfo, Address, Bytecode, B256, U256,
};

use crate::{cache::ReadCaches, BopDbRead, Error};

pub type ProviderReadOnly = DatabaseProviderRO<Arc<DatabaseEnv>, NodeTypesWithDBAdapter<OpNode, Arc<DatabaseEnv>>>;

/// Database access per-block. This is only valid between database commits. Uses read caching.
#[derive(Clone)]
pub struct BlockDB {
    provider: Arc<ProviderReadOnly>,
    caches: ReadCaches,
}

impl BlockDB {
    pub fn state_root(&self) -> Result<B256, Error> {
        let tx = self.provider.tx_ref();
        StateRoot::new(DatabaseTrieCursorFactory::new(tx), DatabaseHashedCursorFactory::new(tx))
            .root()
            .map_err(Error::RethStateRootError)
    }
}

impl Debug for BlockDB {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str("BlockDB")
    }
}

impl BlockDB {
    pub(super) fn new(
        caches: ReadCaches,
        provider: DatabaseProviderRO<Arc<DatabaseEnv>, NodeTypesWithDBAdapter<OpNode, Arc<DatabaseEnv>>>,
    ) -> Self {
        Self { provider: Arc::new(provider), caches }
    }
}

impl DatabaseRef for BlockDB {
    type Error = Error;

    fn basic_ref(&self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        self.caches.account_info(&address, self.provider.tx_ref())
    }

    fn code_by_hash_ref(&self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        let code = self.provider.tx_ref().get::<Bytecodes>(code_hash).map_err(Error::ReadTransactionError)?;
        Ok(code.unwrap_or_default().0)
    }

    fn storage_ref(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
        self.caches.storage(&(address, index), self.provider.tx_ref())
    }

    fn block_hash_ref(&self, number: u64) -> Result<B256, Self::Error> {
        let hash = self.provider.tx_ref().get::<CanonicalHeaders>(number).map_err(Error::ReadTransactionError)?;
        Ok(hash.unwrap_or_default())
    }
}

impl Database for BlockDB {
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

impl BopDbRead for BlockDB {
    fn get_nonce(&self, address: Address) -> u64 {
        self.basic_ref(address).ok().flatten().map(|acc| acc.nonce).unwrap_or_default()
    }

    fn calculate_state_root(&self, bundle_state: &BundleState) -> Result<(B256, TrieUpdates), Error> {
        let latest_state = LatestStateProviderRef::new(self.provider.as_ref());
        let hashed_state = latest_state.hashed_post_state(bundle_state);
        latest_state.state_root_with_updates(hashed_state).map_err(Error::ProviderError)
    }

    /// Returns the highest block number in the canonical chain.
    /// Returns 0 if the database is empty.
    fn block_number(&self) -> Result<u64, Error> {
        self.provider.tx_ref().cursor_read::<CanonicalHeaders>()?.last()?.map_or(Ok(0), |(num, _)| Ok(num))
    }

    fn unique_hash(&self) -> B256 {
        todo!()
    }
}
