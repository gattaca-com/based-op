use std::{
    fmt::{Debug, Formatter},
    sync::Arc,
};

use reth_db::{Bytecodes, CanonicalHeaders, Database, DatabaseEnv, PlainAccountState, PlainStorageState};
use reth_db_api::{cursor::DbDupCursorRO, transaction::DbTx};
use reth_node_ethereum::EthereumNode;
use reth_node_types::NodeTypesWithDBAdapter;
use reth_provider::ProviderFactory;
use revm_primitives::{
    db::{DatabaseCommit, DatabaseRef},
    Account, AccountInfo, Address, Bytecode, HashMap, B256, U256,
};

mod error;
mod init;

pub use error::Error;
pub use init::init_database;

/// Database trait for all DB operations.
pub trait BopDB:
    DatabaseRef<Error: Debug> + DatabaseCommit + BopDbRead + Send + Sync + 'static + Clone + Debug
{
}
impl<T> BopDB for T where
    T: BopDbRead + DatabaseRef<Error: Debug> + DatabaseCommit + Send + Sync + 'static + Clone + Debug
{
}

/// Database read functions
pub trait BopDbRead {
    fn get_nonce(&self, address: Address) -> u64;
}

#[derive(Clone)]
pub struct DB {
    provider: ProviderFactory<NodeTypesWithDBAdapter<EthereumNode, Arc<DatabaseEnv>>>,
}

impl Debug for DB {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str("DB")
    }
}

impl DatabaseRef for DB {
    type Error = error::Error;

    fn basic_ref(&self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        let tx = self.provider.db_ref().tx().map_err(Error::ReadTransactionError)?;
        tx.get::<PlainAccountState>(address)
            .map(|opt| opt.map(|account| account.into()))
            .map_err(Error::ReadTransactionError)
    }

    fn code_by_hash_ref(&self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        let tx = self.provider.db_ref().tx().map_err(Error::ReadTransactionError)?;
        let code = tx.get::<Bytecodes>(code_hash).map_err(Error::ReadTransactionError)?;
        Ok(code.unwrap_or_default().0)
    }

    fn storage_ref(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
        let tx = self.provider.db_ref().tx().map_err(Error::ReadTransactionError)?;
        let mut cursor = tx.cursor_dup_read::<PlainStorageState>().map_err(Error::ReadTransactionError)?;
        let entry = cursor.seek_by_key_subkey(address, index.into()).map_err(Error::ReadTransactionError)?;
        Ok(entry.map(|e| e.value).unwrap_or_default())
    }

    fn block_hash_ref(&self, number: u64) -> Result<B256, Self::Error> {
        let tx = self.provider.db_ref().tx().map_err(Error::ReadTransactionError)?;
        let hash = tx.get::<CanonicalHeaders>(number).map_err(Error::ReadTransactionError)?;
        Ok(hash.unwrap_or_default())
    }
}

impl DatabaseCommit for DB {
    fn commit(&mut self, _changes: HashMap<Address, Account>) {
        todo!()
    }
}

impl BopDbRead for DB {
    fn get_nonce(&self, address: Address) -> u64 {
        self.basic_ref(address).ok().flatten().map(|acc| acc.nonce).unwrap_or_default()
    }
}
