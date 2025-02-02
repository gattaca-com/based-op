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

pub mod alloy_db;
mod block;
mod cache;
mod error;
mod init;
mod util;

pub use error::Error;
pub use init::init_database;
pub use util::state_changes_to_bundle_state;

use crate::{block::BlockDB, cache::ReadCaches};

/// Database trait for all DB operations.
pub trait BopDB: DatabaseCommit + Send + Sync + 'static + Clone + Debug {
    type ReadOnly: BopDbRead;

    /// Returns a read-only database.
    fn readonly(&self) -> Result<Self::ReadOnly, Error>;
}

/// Database read functions
pub trait BopDbRead: DatabaseRef<Error: Debug> + Send + Sync + 'static + Clone + Debug {
    /// Returns the current `nonce` value for the account with the specified address. Zero is
    /// returned if no account is found.
    fn get_nonce(&self, address: Address) -> u64;

    /// Calculate the state root with the provided `BundleState` overlaid on the latest DB state.
    fn calculate_state_root(&self, bundle_state: &BundleState) -> Result<(B256, TrieUpdates), Error>;

    /// Returns the current block head number.
    fn block_number(&self) -> Result<u64, Error>;
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
