use std::sync::Arc;

use reth_db::{Bytecodes, CanonicalHeaders, DatabaseEnv, PlainAccountState, PlainStorageState};
use reth_db_api::{cursor::DbDupCursorRO, transaction::DbTx, Database};
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
pub trait BopDB: DatabaseRef + DatabaseCommit + Send + Sync + 'static {}

#[derive(Clone)]
pub struct DB {
    provider: ProviderFactory<NodeTypesWithDBAdapter<EthereumNode, Arc<DatabaseEnv>>>,
}

impl BopDB for DB {}

impl DatabaseRef for DB {
    #[doc = "The database error type."]
    type Error = error::Error;

    #[doc = " Get basic account information."]
    fn basic_ref(&self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        let tx = self.provider.db_ref().tx().map_err(|e| Error::ReadTransactionError(e))?;
        tx.get::<PlainAccountState>(address)
            .map(|opt| opt.map(|account| account.into()))
            .map_err(|e| Error::ReadTransactionError(e))
    }

    #[doc = " Get account code by its hash."]
    fn code_by_hash_ref(&self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        let tx = self.provider.db_ref().tx().map_err(|e| Error::ReadTransactionError(e))?;
        let code = tx.get::<Bytecodes>(code_hash).map_err(|e| Error::ReadTransactionError(e))?;
        Ok(code.unwrap_or_default().0.into())
    }

    #[doc = " Get storage value of address at index."]
    fn storage_ref(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
        let tx = self.provider.db_ref().tx().map_err(|e| Error::ReadTransactionError(e))?;
        let mut cursor = tx.cursor_dup_read::<PlainStorageState>().map_err(|e| Error::ReadTransactionError(e))?;
        let entry = cursor.seek_by_key_subkey(address, index.into()).map_err(|e| Error::ReadTransactionError(e))?;
        Ok(entry.map(|e| e.value).unwrap_or_default())
    }

    #[doc = " Get block hash by block number."]
    fn block_hash_ref(&self, number: u64) -> Result<B256, Self::Error> {
        let tx = self.provider.db_ref().tx().map_err(|e| Error::ReadTransactionError(e))?;
        let hash = tx.get::<CanonicalHeaders>(number).map_err(|e| Error::ReadTransactionError(e))?;
        Ok(hash.unwrap_or_default())
    }
}

impl DatabaseCommit for DB {
    fn commit(&mut self, _changes: HashMap<Address, Account>) {
        todo!()
    }
}
