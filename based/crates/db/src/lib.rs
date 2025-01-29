use revm_primitives::{db::DatabaseRef, AccountInfo, Address, Bytecode, B256, U256};

#[derive(Clone, Debug, Default)]
pub struct DbStub {}

impl DatabaseRef for DbStub {
    #[doc = "The database error type."]
    type Error = &'static str;

    #[doc = " Get basic account information."]
    fn basic_ref(&self, _address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        Ok(Some(AccountInfo::default()))
    }

    #[doc = " Get account code by its hash."]
    fn code_by_hash_ref(&self, _code_hash: B256) -> Result<Bytecode, Self::Error> {
        todo!()
    }

    #[doc = " Get storage value of address at index."]
    fn storage_ref(&self, _address: Address, _index: U256) -> Result<U256, Self::Error> {
        todo!()
    }

    #[doc = " Get block hash by block number."]
    fn block_hash_ref(&self, _number: u64) -> Result<B256, Self::Error> {
        todo!()
    }
}

impl DbStub {
    pub fn new() -> Self {
        Self {}
    }

    pub fn get_nonce(&self, _address: Address) -> u64 {
        0
    }
}
