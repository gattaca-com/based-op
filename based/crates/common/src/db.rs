use std::{
    fmt::{Debug, Display},
    io,
    ops::Deref,
    sync::Arc,
};

use alloy_primitives::{BlockNumber, B256};
use auto_impl::auto_impl;
use op_alloy_rpc_types::OpTransactionReceipt;
use parking_lot::RwLock;
use reth_optimism_primitives::{OpBlock, OpReceipt};
use reth_primitives::BlockWithSenders;
use reth_provider::BlockExecutionOutput;
use reth_storage_errors::{db::DatabaseError, provider::ProviderError};
use reth_trie_common::updates::TrieUpdates;
use revm::db::{BundleState, CacheDB};
use revm_primitives::{
    db::{Database, DatabaseCommit, DatabaseRef},
    AccountInfo, Address, Bytecode, EvmState, U256,
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
    #[error("Reth state root error: {0}")]
    RethStateRootError(#[from] reth_execution_errors::StateRootError),
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
            Error::RethStateRootError(e) => ProviderError::Database(DatabaseError::Other(e.to_string())),
        }
    }
}

/// Database trait for all DB operations.
#[auto_impl(&, Arc)]
pub trait BopDB: Database<Error: Into<ProviderError> + Display> + Send + Sync + 'static + Clone + Debug {
    type ReadOnly: BopDbRead + Database<Error: Into<ProviderError> + Display>;

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
#[auto_impl(&, Arc)]
pub trait BopDbRead:
    DatabaseRef<Error: Debug + Display + Into<ProviderError>> + Send + Sync + 'static + Clone + Debug
{
    /// Calculate the state root with the provided `BundleState` overlaid on the latest DB state.
    fn calculate_state_root(&self, bundle_state: &BundleState) -> Result<(B256, TrieUpdates), Error>;

    /// Returns the head block number, ie. the highest block number on the chain
    fn head_block_number(&self) -> Result<u64, Error>;
}

impl<DbRead: BopDbRead> BopDbRead for CacheDB<DbRead> {
    fn calculate_state_root(&self, bundle_state: &BundleState) -> Result<(B256, TrieUpdates), Error> {
        self.db.calculate_state_root(bundle_state)
    }

    fn head_block_number(&self) -> Result<u64, Error> {
        self.db.head_block_number()
    }
}

/// DB That adds chunks on top of last on chain block
#[derive(Clone, Debug)]
pub struct DBFrag<Db> {
    pub db: Arc<RwLock<CacheDB<Db>>>,
    /// Unique identifier for the state in the db
    state_id: u64,
    // Block number for block that is currently being sorted
    curr_block_number: u64,
}

impl<Db: BopDbRead> DBFrag<Db> {
    pub fn commit(&mut self, db: Arc<DBSorting<Db>>) {
        let guard = self.db.write();
        todo!();
        self.state_id = rand::random()
    }

    pub fn clear(&mut self) {
        let guard = self.db.write();
        self.curr_block_number = guard.head_block_number().unwrap() + 1;
        todo!();
        self.state_id = rand::random()
    }

    pub fn get_nonce(&self, address: Address) -> Result<u64, Error> {
        self.basic_ref(address)
            .map(|acc| acc.map(|acc| acc.nonce).unwrap_or_default())
            .map_err(|_| Error::Other("failed to get nonce".to_string()))
    }

    pub fn get_balance(&self, address: Address) -> Result<U256, Error> {
        self.basic_ref(address)
            .map(|acc| acc.map(|acc| acc.balance).unwrap_or_default())
            .map_err(|_| Error::Other("failed to get nonce".to_string()))
    }

    pub fn state_id(&self) -> u64 {
        self.state_id
    }

    pub fn curr_block_number(&self) -> Result<u64, Error> {
        Ok(self.curr_block_number)
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

impl<Db: BopDbRead> Database for DBFrag<Db> {
    type Error = <Db as DatabaseRef>::Error;

    fn basic(&mut self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        self.db.read().basic_ref(address)
    }

    fn code_by_hash(&mut self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        self.db.read().code_by_hash_ref(code_hash)
    }

    fn storage(&mut self, address: Address, index: U256) -> Result<U256, Self::Error> {
        self.db.read().storage_ref(address, index)
    }

    fn block_hash(&mut self, number: u64) -> Result<B256, Self::Error> {
        self.db.read().block_hash_ref(number)
    }
}

impl<Db: BopDbRead> BopDbRead for DBFrag<Db> {
    fn calculate_state_root(&self, bundle_state: &BundleState) -> Result<(B256, TrieUpdates), Error> {
        self.db.read().calculate_state_root(bundle_state)
    }

    fn head_block_number(&self) -> Result<u64, Error> {
        Ok(self.curr_block_number - 1)
    }
}

impl<Db: BopDbRead> From<Db> for DBFrag<Db> {
    fn from(value: Db) -> Self {
        let curr_block_number = value.head_block_number().unwrap() + 1;
        Self { db: Arc::new(RwLock::new(CacheDB::new(value))), state_id: rand::random(), curr_block_number }
    }
}

/// DB That is used when sorting a new frag

#[derive(Clone, Debug)]
pub struct DBSorting<Db> {
    db: CacheDB<DBFrag<Db>>,
    state_id: u64,
}

impl<Db> DBSorting<Db> {
    pub fn new(frag_db: DBFrag<Db>) -> Self {
        Self { db: CacheDB::new(frag_db), state_id: rand::random() }
    }

    pub fn state_id(&self) -> u64 {
        self.state_id
    }
}

impl<Db> DBSorting<Db> {
    pub fn commit(&mut self, state: EvmState) {
        self.db.commit(state);
        self.state_id = rand::random()
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

impl<Db: BopDbRead> Database for DBSorting<Db> {
    type Error = <Db as DatabaseRef>::Error;

    fn basic(&mut self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        self.db.basic(address)
    }

    fn code_by_hash(&mut self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        self.db.code_by_hash(code_hash)
    }

    fn storage(&mut self, address: Address, index: U256) -> Result<U256, Self::Error> {
        self.db.storage_ref(address, index)
    }

    fn block_hash(&mut self, number: u64) -> Result<B256, Self::Error> {
        self.db.block_hash_ref(number)
    }
}

impl<DbRead: BopDbRead> BopDbRead for DBSorting<DbRead> {
    fn calculate_state_root(&self, bundle_state: &BundleState) -> Result<(B256, TrieUpdates), Error> {
        self.db.calculate_state_root(bundle_state)
    }

    fn head_block_number(&self) -> Result<u64, Error> {
        self.db.head_block_number()
    }
}
