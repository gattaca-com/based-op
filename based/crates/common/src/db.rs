use std::{
    fmt::{Debug, Formatter},
    ops::Deref,
    sync::Arc,
};

use alloy_primitives::B256;
use auto_impl::auto_impl;
use op_alloy_rpc_types::OpTransactionReceipt;
use parking_lot::RwLock;
use reth_db::DatabaseEnv;
use reth_node_ethereum::EthereumNode;
use reth_node_types::NodeTypesWithDBAdapter;
use reth_optimism_primitives::OpBlock;
use reth_provider::ProviderFactory;
use reth_trie_common::updates::TrieUpdates;
use revm::db::{BundleState, CacheDB};
use revm_primitives::{
    db::{DatabaseCommit, DatabaseRef},
    Account, AccountInfo, Address, Bytecode, EvmState, HashMap, U256,
};

pub mod alloy_db;
mod block;
mod cache;
mod error;
mod init;
mod util;

use block::BlockDB;
use cache::ReadCaches;
pub use error::Error;
pub use init::init_database;
pub use util::state_changes_to_bundle_state;

/// Database trait for all DB operations.
#[auto_impl(&, Arc)]
pub trait BopDB: DatabaseCommit + Send + Sync + 'static + Clone + Debug {
    type ReadOnly: BopDbRead;

    /// Returns a read-only database.
    fn readonly(&self) -> Result<Self::ReadOnly, Error>;
}

/// Database read functions
#[auto_impl(&, Arc)]
pub trait BopDbRead: DatabaseRef<Error: Debug> + Send + Sync + 'static + Clone + Debug {
    /// Calculate the state root with the provided `BundleState` overlaid on the latest DB state.
    fn calculate_state_root(&self, bundle_state: &BundleState) -> Result<(B256, TrieUpdates), Error>;

    /// Get a unique hash of current state
    fn unique_hash(&self) -> B256;
}

impl<DbRead: BopDbRead> BopDbRead for CacheDB<DbRead> {
    fn calculate_state_root(&self, bundle_state: &BundleState) -> Result<(B256, TrieUpdates), Error> {
        self.db.calculate_state_root(bundle_state)
    }

    fn unique_hash(&self) -> B256 {
        self.db.unique_hash()
    }
}

/// DB That adds chunks on top of last on chain block
#[derive(Clone, Debug)]
pub struct DBFrag<Db> {
    db: Arc<RwLock<CacheDB<Db>>>,
    unique_hash: B256,
}

impl<Db: DatabaseRef> DatabaseRef for DBFrag<Db> {
    type Error = <Db as DatabaseRef>::Error;

    fn basic_ref(&self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        self.db.read().basic_ref(address)
    }

    fn code_by_hash_ref(&self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        self.db.read().code_by_hash_ref(code_hash)
    }

    fn storage_ref(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
        self.db.read().storage_ref(address, index)
    }

    fn block_hash_ref(&self, number: u64) -> Result<B256, Self::Error> {
        self.db.read().block_hash_ref(number)
    }
}

impl<Db: DatabaseRef> DBFrag<Db> {
    pub fn get_nonce(&self, _address: Address) -> Result<u64, Error> {
        todo!()
        // self.basic_ref(address).map(|info| info.map(|info| info.nonce).unwrap_or_default())
    }

    pub fn get_balance(&self, _address: Address) -> Result<U256, Error> {
        todo!()
        // self.basic_ref(address).map(|info| info.map(|info| info.balance).unwrap_or_default())
    }

    pub fn get_latest_block_number(&self) -> Result<u64, Error> {
        todo!()
    }

    pub fn get_latest_block(&self) -> Result<OpBlock, Error> {
        todo!()
    }

    pub fn get_latest_block_hash(&self) -> Result<B256, Error> {
        todo!()
    }

    pub fn get_block_by_number(&self, _number: u64) -> Result<OpBlock, Error> {
        todo!()
    }

    pub fn get_block_by_hash(&self, _hash: B256) -> Result<OpBlock, Error> {
        todo!()
    }

    pub fn get_transaction_receipt(&self, _hash: B256) -> Result<OpTransactionReceipt, Error> {
        todo!()
    }
}

impl<Db: BopDbRead> BopDbRead for DBFrag<Db> {
    fn calculate_state_root(&self, bundle_state: &BundleState) -> Result<(B256, TrieUpdates), Error> {
        self.db.read().calculate_state_root(bundle_state)
    }

    fn unique_hash(&self) -> B256 {
        self.unique_hash
    }
}

impl<Db: BopDbRead> From<Db> for DBFrag<Db> {
    fn from(value: Db) -> Self {
        Self { db: Arc::new(RwLock::new(CacheDB::new(value))), unique_hash: B256::random() }
    }
}

/// DB That is used when sorting a new frag
#[derive(Clone, Debug)]
pub struct DBSorting<Db> {
    db: CacheDB<DBFrag<Db>>,
    unique_hash: B256,
}

impl<Db> DBSorting<Db> {
    pub fn commit(&mut self, state: EvmState) {
        self.db.commit(state);
        self.unique_hash = B256::random()
    }
}

impl<Db: BopDbRead> From<DBFrag<Db>> for DBSorting<Db> {
    fn from(value: DBFrag<Db>) -> Self {
        Self { db: CacheDB::new(value), unique_hash: B256::random() }
    }
}
impl<Db> Deref for DBSorting<Db> {
    type Target = CacheDB<DBFrag<Db>>;

    fn deref(&self) -> &Self::Target {
        &self.db
    }
}
impl<DbRead: DatabaseRef> DatabaseRef for DBSorting<DbRead> {
    type Error = DbRead::Error;

    fn basic_ref(&self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        self.db.basic_ref(address)
    }

    fn code_by_hash_ref(&self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        self.db.code_by_hash_ref(code_hash)
    }

    fn storage_ref(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
        self.db.storage_ref(address, index)
    }

    fn block_hash_ref(&self, number: u64) -> Result<B256, Self::Error> {
        self.db.block_hash_ref(number)
    }
}
impl<DbRead: BopDbRead> BopDbRead for DBSorting<DbRead> {
    fn calculate_state_root(&self, bundle_state: &BundleState) -> Result<(B256, TrieUpdates), Error> {
        self.db.calculate_state_root(bundle_state)
    }

    fn unique_hash(&self) -> B256 {
        self.unique_hash
    }
}

pub struct DB {
    factory: ProviderFactory<NodeTypesWithDBAdapter<EthereumNode, Arc<DatabaseEnv>>>,
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
