use std::{
    collections::hash_map::Entry,
    fmt::{Debug, Display},
    io,
    ops::Deref,
    sync::Arc,
};

use alloy_consensus::constants::KECCAK_EMPTY;
use alloy_primitives::{map::HashMap, BlockNumber, B256};
use auto_impl::auto_impl;
use parking_lot::RwLock;
use reth_optimism_primitives::{OpBlock, OpReceipt};
use reth_primitives::BlockWithSenders;
use reth_provider::BlockExecutionOutput;
use reth_storage_errors::{db::DatabaseError, provider::ProviderError};
use reth_trie_common::updates::TrieUpdates;
use revm::db::{BundleState, CacheDB};
use revm_primitives::{
    db::{Database, DatabaseCommit, DatabaseRef},
    keccak256, Account, AccountInfo, AccountStatus, Address, Bytecode, EvmState, U256,
};
use thiserror::Error;

use crate::transaction::SimulatedTx;

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
pub trait BopDB:
    Database<Error: Into<ProviderError> + Display> + Send + Sync + 'static + Clone + Debug
{
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
    pub db: Arc<RwLock<CacheDB<Db>>>,
    pub unique_hash: B256,
}

impl<Db: BopDbRead> DBFrag<Db> {
    pub fn commit<'a>(&mut self, txs: impl Iterator<Item = &'a SimulatedTx>) {
        let mut guard = self.db.write();

        for t in txs {
            guard.commit(t.clone_state())
        }

        self.unique_hash = B256::random()
    }

    pub fn reset(&mut self) {
        let mut guard = self.db.write();
        guard.accounts.clear();
        guard.contracts.clear();
        guard.logs.clear();
        guard.block_hashes.clear();
        self.unique_hash = B256::random()
    }

    pub fn state_root(&self, state_changes: HashMap<Address, Account>) -> B256 {
        let r = self.db.read();
        let bundle_state = state_changes_to_bundle_state(&r.db, state_changes).expect("couldn't create bundle state");
        self.calculate_state_root(&bundle_state).expect("couldn't calculate state root").0
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
    pub db: CacheDB<DBFrag<Db>>,
    pub unique_hash: B256,
}

impl<Db> DBSorting<Db> {
    pub fn new(frag_db: DBFrag<Db>) -> Self {
        Self { db: CacheDB::new(frag_db), unique_hash: B256::random() }
    }
}

impl<Db> DBSorting<Db> {
    pub fn commit(&mut self, state: EvmState) {
        self.db.commit(state);
        self.unique_hash = B256::random()
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

/// Converts cached state in a `CachedDB` into `BundleState`
pub fn state_changes_to_bundle_state<D: DatabaseRef>(
    db: &D,
    changes: HashMap<Address, Account>,
) -> Result<BundleState, D::Error> {
    let mut bundle_state = BundleState::builder(0..=2);

    for (address, account) in changes {
        if let Some(original_account_info) = db.basic_ref(address)? {
            bundle_state = bundle_state.state_original_account_info(address, original_account_info);
        }
        bundle_state = bundle_state.state_present_account_info(address, account.info);
        bundle_state = bundle_state.state_storage(
            address,
            account.storage.into_iter().map(|(i, s)| (i, (s.original_value, s.present_value))).collect(),
        );
    }
    Ok(bundle_state.build())
}

// This function is used to flatten a vector of state changes into a single HashMap.
// The idea is to merge the changes that happened to the same account across multiple transactions
// into a single "Account" struct that represents the final state of the accounts after all
// transactions.
pub fn flatten_state_changes(state_changes: Vec<HashMap<Address, Account>>) -> HashMap<Address, Account> {
    let mut flat_state_change_map: HashMap<Address, Account> = HashMap::default();

    for tx_state_changes in state_changes {
        update_state_changes(&mut flat_state_change_map, tx_state_changes);
    }

    flat_state_change_map
}

// This function is used to add state changes to an existing map of state changes.
// The idea is to merge the changes that happened to the same account across multiple transactions
// into a single "Account" struct that represents the final state of the accounts after all
// transactions.
pub fn update_state_changes(
    original_state_changes: &mut HashMap<Address, Account>,
    tx_state_changes: HashMap<Address, Account>,
) {
    for (address, mut new_account) in tx_state_changes {
        if !new_account.is_touched() {
            continue;
        }

        match original_state_changes.entry(address) {
            Entry::Occupied(mut entry) => {
                let db_account = entry.get_mut();
                let is_newly_created = new_account.is_created();

                // Set the storage
                if new_account.is_selfdestructed() {
                    db_account.storage.clear();
                } else if is_newly_created || !db_account.is_selfdestructed() {
                    db_account.storage.extend(new_account.storage);
                }

                if !db_account.is_selfdestructed() || is_newly_created {
                    // Set the info
                    db_account.info = new_account.info;
                    db_account.status = new_account.status;
                }
            }
            Entry::Vacant(entry) => {
                if new_account.is_selfdestructed() {
                    new_account.storage.clear();
                }
                entry.insert(new_account);
            }
        }
    }
}
