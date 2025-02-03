use std::{
    fmt::{Debug, Display},
    io,
};

use alloy_primitives::{BlockNumber, B256};
use reth_optimism_primitives::{OpBlock, OpReceipt};
use reth_primitives::BlockWithSenders;
use reth_provider::BlockExecutionOutput;
use reth_storage_errors::{db::DatabaseError, provider::ProviderError};
use reth_trie_common::updates::TrieUpdates;
use revm::db::BundleState;
use revm_primitives::{
    db::{Database, DatabaseCommit, DatabaseRef},
    Address,
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("Directory not readable: {0}, {1}")]
    DirNotReadable(String, io::Error),
    #[error("Directory not writable: {0}, {1}")]
    DirNotWritable(String, io::Error),
    #[error("Database could not be initialised: {0}")]
    DatabaseInitialisationError(String),
    #[error(transparent)]
    ProviderError(#[from] ProviderError),
    #[error("Read transaction error: {0}")]
    ReadTransactionError(#[from] DatabaseError),
    #[error("{0}")]
    Other(String),
    #[error("State root mismatch: {0}")]
    StateRootError(BlockNumber),
}

impl From<Error> for ProviderError {
    fn from(value: Error) -> Self {
        match value {
            Error::DirNotReadable(path, _) => ProviderError::FsPathError(path),
            Error::DirNotWritable(path, _) => ProviderError::FsPathError(path),
            Error::DatabaseInitialisationError(e) => ProviderError::Database(DatabaseError::Other(e)),
            Error::ProviderError(e) => e,
            Error::ReadTransactionError(e) => ProviderError::Database(e),
            Error::Other(e) => ProviderError::Database(DatabaseError::Other(e)),
            Error::StateRootError(e) => ProviderError::Database(DatabaseError::Other(e.to_string())),
        }
    }
}

/// Database trait for all DB operations.
pub trait BopDB: DatabaseCommit + Send + Sync + 'static + Clone + Debug {
    type ReadOnly: BopDbRead;

    /// Returns a read-only database.
    fn readonly(&self) -> Result<Self::ReadOnly, Error>;

    fn commit_block(
        &self,
        block: &BlockWithSenders<OpBlock>,
        block_execution_output: BlockExecutionOutput<OpReceipt>,
    ) -> Result<(), Error>;

    fn commit_block_unchecked(
        &self,
        block: &BlockWithSenders<OpBlock>,
        block_execution_output: BlockExecutionOutput<OpReceipt>,
        trie_updates: TrieUpdates,
    ) -> Result<(), Error>;
}

/// Database read functions
pub trait BopDbRead:
    Database<Error: Into<ProviderError> + Display> + DatabaseRef<Error: Debug> + Send + Sync + 'static + Clone + Debug
{
    /// Returns the current `nonce` value for the account with the specified address. Zero is
    /// returned if no account is found.
    fn get_nonce(&self, address: Address) -> u64;

    /// Calculate the state root with the provided `BundleState` overlaid on the latest DB state.
    fn calculate_state_root(&self, bundle_state: &BundleState) -> Result<(B256, TrieUpdates), Error>;

    /// Get a unique hash of current state
    fn unique_hash(&self) -> B256;

    /// Get a unique hash of current state
    fn block_number(&self) -> Result<u64, Error>;
}

impl<DbRead: BopDbRead> BopDbRead for CacheDB<DbRead> {
    fn get_nonce(&self, address: Address) -> u64 {
        self.db.get_nonce(address)
    }

    fn calculate_state_root(&self, bundle_state: &BundleState) -> Result<(B256, TrieUpdates), Error> {
        self.db.calculate_state_root(bundle_state)
    }

    fn unique_hash(&self) -> B256 {
        self.db.unique_hash()
    }

    fn block_number(&self) -> Result<u64, Error> {
        todo!()
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

impl<Db: BopDbRead> BopDbRead for DBFrag<Db> {
    fn get_nonce(&self, address: Address) -> u64 {
        self.db.read().get_nonce(address)
    }

    fn calculate_state_root(&self, bundle_state: &BundleState) -> Result<(B256, TrieUpdates), Error> {
        self.db.read().calculate_state_root(bundle_state)
    }

    fn unique_hash(&self) -> B256 {
        self.unique_hash
    }

    fn block_number(&self) -> Result<u64, Error> {
        todo!()
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
    fn get_nonce(&self, address: Address) -> u64 {
        self.db.get_nonce(address)
    }

    fn calculate_state_root(&self, bundle_state: &BundleState) -> Result<(B256, TrieUpdates), Error> {
        self.db.calculate_state_root(bundle_state)
    }

    fn unique_hash(&self) -> B256 {
        self.unique_hash
    }

    fn block_number(&self) -> Result<u64, Error> {
        todo!()
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

impl DatabaseRef for DB {
    type Error = <<DB as BopDB>::ReadOnly as DatabaseRef>::Error;

    fn basic_ref(&self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        todo!()
    }

    fn code_by_hash_ref(&self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        todo!()
    }

    fn storage_ref(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
        todo!()
    }

    fn block_hash_ref(&self, number: u64) -> Result<B256, Self::Error> {
        todo!()
    }
}

impl BopDbRead for DB {
    fn get_nonce(&self, address: Address) -> u64 {
        todo!()
    }

    fn calculate_state_root(&self, bundle_state: &BundleState) -> Result<(B256, TrieUpdates), Error> {
        todo!()
    }

    fn unique_hash(&self) -> B256 {
        todo!()
    }

    fn block_number(&self) -> Result<u64, Error> {
        todo!()
    }
}

pub enum DBError {}
// these two are needed for blocksync
impl std::fmt::Display for DBError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        todo!()
    }
}
impl Into<ProviderError> for DBError {
    fn into(self) -> ProviderError {
        todo!()
    }
}
// also this
impl Database for DB {
    type Error = DBError;

    fn basic(&mut self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        todo!()
    }

    fn code_by_hash(&mut self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        todo!()
    }

    fn storage(&mut self, address: Address, index: U256) -> Result<U256, Self::Error> {
        todo!()
    }

    fn block_hash(&mut self, number: u64) -> Result<B256, Self::Error> {
        todo!()
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
