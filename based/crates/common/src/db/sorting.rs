use std::sync::Arc;

use alloy_primitives::B256;
use parking_lot::RwLock;
use reth_trie_common::updates::TrieUpdates;
use revm::db::{BundleState, CacheDB};
use revm_primitives::{
    db::{Database, DatabaseCommit, DatabaseRef},
    AccountInfo, Address, Bytecode, EvmState, U256,
};

use super::{DBFrag, DatabaseRead, Error};

/// DB That is used when sorting a new frag
/// Thread safe
#[derive(Clone, Debug)]
pub struct DBSorting<Db> {
    db: Arc<RwLock<CacheDB<DBFrag<Db>>>>,
    state_id: u64,
}

impl<Db> DBSorting<Db> {
    pub fn new(frag_db: DBFrag<Db>) -> Self {
        Self { db: Arc::new(RwLock::new(CacheDB::new(frag_db))), state_id: rand::random() }
    }

    pub fn state_id(&self) -> u64 {
        self.state_id
    }
}

impl<Db> DBSorting<Db> {
    pub fn commit(&mut self, state: EvmState) {
        self.db.write().commit(state);
        self.state_id = rand::random()
    }
}

// impl<Db> Deref for DBSorting<Db> {
//     type Target = CacheDB<DBFrag<Db>>;

//     fn deref(&self) -> &Self::Target {
//         &self.db.read()
//     }
// }

impl<Db: DatabaseRef> DatabaseRef for DBSorting<Db> {
    type Error = Db::Error;

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

impl<Db: DatabaseRef> Database for DBSorting<Db> {
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

impl<DbRead: DatabaseRead> DatabaseRead for DBSorting<DbRead> {
    fn calculate_state_root(&self, bundle_state: &BundleState) -> Result<(B256, TrieUpdates), Error> {
        self.db.read().calculate_state_root(bundle_state)
    }

    fn head_block_number(&self) -> Result<u64, Error> {
        self.db.read().head_block_number()
    }

    fn head_block_hash(&self) -> Result<B256, Error> {
        self.db.read().head_block_hash()
    }
}
