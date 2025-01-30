use std::{
    fmt::{Debug, Formatter},
    sync::Arc,
};

use alloy_primitives::B256;
use parking_lot::RwLock;
use reth_db::DatabaseEnv;
use reth_node_ethereum::EthereumNode;
use reth_node_types::NodeTypesWithDBAdapter;
use reth_provider::ProviderFactory;
use reth_trie_common::updates::TrieUpdates;
use revm::db::BundleState;
use revm_primitives::{
    db::{DatabaseCommit, DatabaseRef},
    Account, Address, HashMap,
};

mod block;
mod cache;
mod error;
mod init;
mod util;

pub use error::Error;
pub use init::init_database;

use crate::{block::BlockDB, cache::ReadCaches};

/// Database trait for all DB operations.
pub trait BopDB: DatabaseCommit + Send + Sync + 'static + Clone + Debug {
    /// Returns a read-only database that is valid for the current block only.
    fn block_db_readonly(&self) -> Result<Arc<impl BopDbRead>, Error>;
}

/// Database read functions
pub trait BopDbRead: DatabaseRef<Error: Debug> {
    fn get_nonce(&self, address: Address) -> u64;

    /// Calculate the state root with the provided `BundleState` overlaid on the latest DB state.
    fn calculate_state_root(&self, bundle_state: &BundleState) -> Result<(B256, TrieUpdates), Error>;
}

pub struct DB {
    factory: ProviderFactory<NodeTypesWithDBAdapter<EthereumNode, Arc<DatabaseEnv>>>,
    caches: ReadCaches,
    block: RwLock<Option<Arc<BlockDB>>>,
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
    fn block_db_readonly(&self) -> Result<Arc<impl BopDbRead>, Error> {
        if let Some(block) = self.block.read().as_ref().cloned() {
            return Ok(block);
        }

        let block = Arc::new(BlockDB::new(self.caches.clone(), self.factory.provider().map_err(Error::ProviderError)?));
        self.block.write().replace(block.clone());
        Ok(block)
    }
}

impl DatabaseCommit for DB {
    fn commit(&mut self, _changes: HashMap<Address, Account>) {
        todo!()
    }
}
