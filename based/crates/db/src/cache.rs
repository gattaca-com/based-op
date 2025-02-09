use std::sync::Arc;

use alloy_primitives::{Address, B256, U256};
use moka::sync::{Cache, CacheBuilder};
use reth_db::{
    mdbx::{tx::Tx, RO},
    PlainAccountState, PlainStorageState,
};
use reth_db_api::{cursor::DbDupCursorRO, transaction::DbTx};
use revm::db::BundleState;
use revm_primitives::AccountInfo;

use crate::Error;

/// Caches used to accelerate database reads. Cache entries are retained according to LRU policy.
/// On database commits, corresponding entries in the caches are invalidated.
#[derive(Clone)]
pub(super) struct ReadCaches {
    account_info: Cache<Address, Option<AccountInfo>>,
    storage: Cache<(Address, U256), U256>,
}

impl ReadCaches {
    pub(super) fn new(max_cached_accounts: u64, max_cached_storage: u64) -> Self {
        let account_info = CacheBuilder::new(max_cached_accounts).build();
        let storage = CacheBuilder::new(max_cached_storage).build();
        Self { account_info, storage }
    }

    pub(super) fn account_info(&self, address: &Address, tx: &Tx<RO>) -> Result<Option<AccountInfo>, Error> {
        self.account_info
            .entry_by_ref(address)
            .or_try_insert_with(|| {
                tx.get::<PlainAccountState>(*address)
                    .map(|opt| opt.map(|account| account.into()))
                    .map_err(Error::ReadTransactionError)
            })
            .map(|entry| entry.into_value())
            .map_err(|e| Arc::into_inner(e).unwrap_or_else(|| Error::Other("Couldn't unwrap Arced error".to_string())))
    }

    pub(super) fn storage(&self, key: &(Address, U256), tx: &Tx<RO>) -> Result<U256, Error> {
        self.storage
            .entry_by_ref(key)
            .or_try_insert_with(|| {
                let (address, index) = key;
                let mut cursor = tx.cursor_dup_read::<PlainStorageState>().map_err(Error::ReadTransactionError)?;
                let storage_key = B256::from(index.to_be_bytes());
                match cursor.seek_by_key_subkey(*address, storage_key).map_err(Error::ReadTransactionError)? {
                    Some(entry) if entry.key == storage_key => Ok(entry.value),
                    _ => Ok(U256::default()),
                }
            })
            .map(|entry| entry.into_value())
            .map_err(|e| Arc::into_inner(e).unwrap())
    }

    pub(super) fn update(&self, changes: &BundleState) {
        for (address, account) in changes.state() {
            if account.was_destroyed() {
                self.account_info.remove(address);
            } else {
                self.account_info.insert(*address, account.info.clone());
                for (index, slot) in &account.storage {
                    self.storage.insert((*address, *index), slot.present_value());
                }
            }
        }
        self.account_info.run_pending_tasks();
        self.storage.run_pending_tasks();
    }
}
