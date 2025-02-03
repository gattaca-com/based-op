use alloy_primitives::map::foldhash::HashMap;

use revm::{
    db::DatabaseRef,
    primitives::{AccountInfo, Address, B256, U256},
    Database,
};

/// GenesisDB is an empty db that is initialised with the "genesis_alloc"
#[derive(Clone, Debug)]
pub struct GenesisDB {
    state: HashMap<Address, AccountInfo>,
}

impl GenesisDB {
    pub fn new(genesis_alloc: HashMap<Address, AccountInfo>) -> Self {
        GenesisDB { state: genesis_alloc }
    }
}

impl Database for GenesisDB {
    type Error = &'static str;

    fn basic(&mut self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        Ok(self.state.get(&address).cloned())
    }

    fn code_by_hash(&mut self, _code_hash: B256) -> Result<revm::primitives::Bytecode, Self::Error> {
        panic!("Should not be called. Code is already loaded");
    }

    fn storage(&mut self, _address: Address, _index: U256) -> Result<U256, Self::Error> {
        Ok(U256::ZERO)
    }

    fn block_hash(&mut self, _number: u64) -> Result<B256, Self::Error> {
        Ok(B256::ZERO)
    }
}

impl DatabaseRef for GenesisDB {
    type Error = &'static str;

    fn basic_ref(&self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        Ok(self.state.get(&address).cloned())
    }

    fn code_by_hash_ref(&self, _code_hash: B256) -> Result<revm::primitives::Bytecode, Self::Error> {
        panic!("Should not be called. Code is already loaded");
    }

    fn storage_ref(&self, _address: Address, _index: U256) -> Result<U256, Self::Error> {
        Ok(U256::ZERO)
    }

    fn block_hash_ref(&self, _number: u64) -> Result<B256, Self::Error> {
        Ok(B256::ZERO)
    }
}

