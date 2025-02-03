use alloy_primitives::map::foldhash::HashMap;
use bop_db::{BopDB, BopDbRead, Error};
use rand::prelude::*;
use reth_storage_errors::provider::ProviderError;
use reth_primitives::{BlockWithSenders};
use reth_optimism_primitives::{OpBlock, OpReceipt};
use reth_provider::{
    BlockExecutionOutput, ExecutionOutcome, LatestStateProviderRef, ProviderFactory, StateWriter, TrieWriter,
};
use reth_trie_common::updates::TrieUpdates;

use revm::{
    db::{BundleState, DatabaseRef},
    primitives::{Account, AccountInfo, Address, Bytecode, B256, KECCAK_EMPTY, U256},
    Database, DatabaseCommit,
};

#[derive(Debug)]
pub struct DBError {}

impl Into<ProviderError> for DBError {
    fn into(self) -> ProviderError {
        todo!()
    }
}

impl std::fmt::Display for DBError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "error")
    }
}

/// GenesisDB is an empty db that is initialised with the "genesis_alloc"
#[derive(Clone, Debug)]
pub struct GenesisDB {
    state: HashMap<Address, AccountInfo>,
}

impl GenesisDB {
    pub fn new(genesis_alloc: HashMap<Address, AccountInfo>) -> Self {
        GenesisDB { state: genesis_alloc }
    }

    pub fn new_with_randoms(n_randoms: usize) -> Self {
        let mut state = HashMap::default();
        for _ in 0..n_randoms {
            let addr1 = Address::random();
            state.insert(addr1, AccountInfo::new(U256::from_limbs([123, 0, 0, 0]), 0, KECCAK_EMPTY, Bytecode::new()));
        }
        Self { state }
    }

    pub fn get_rand_addr(&self) -> &Address {
        let mut rng = rand::rng();

        let n = (0..self.len() - 1).choose(&mut rng).unwrap();
        self.state.keys().nth(n).unwrap()
    }

    pub fn len(&self) -> usize {
        self.state.len()
    }

    pub fn balance(&self, addr: &Address) -> Option<U256> {
        self.state.get(addr).map(|t| t.balance)
    }
}

impl Database for GenesisDB {
    type Error = bop_db::Error;

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
    type Error = DBError;

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

impl BopDB for GenesisDB {
    type ReadOnly = Self;

    #[doc = " Returns a read-only database."]
    fn readonly(&self) -> Result<Self::ReadOnly, Error> {
        Ok(self.clone())
    }

    fn commit_block(
        &self,
        block: &BlockWithSenders<OpBlock>,
        block_execution_output: BlockExecutionOutput<OpReceipt>,
    ) -> Result<(), Error> {
        todo!()
    }

    fn commit_block_unchecked(
        &self,
        block: &BlockWithSenders<OpBlock>,
        block_execution_output: BlockExecutionOutput<OpReceipt>,
        trie_updates: TrieUpdates,
    ) -> Result<(), Error> {
        todo!()
    }
}

impl DatabaseCommit for GenesisDB {
    fn commit(&mut self,changes:HashMap<Address,Account>) {
            todo!()
    }
}

impl BopDbRead for GenesisDB {
    fn get_nonce(&self, address: Address) -> u64 {
        todo!()
    }

    fn calculate_state_root(&self, bundle_state: &BundleState) -> Result<(B256, TrieUpdates), Error> {
        todo!()
    }

    fn unique_hash(&self) -> B256 {
        todo!()
    }

    fn block_number(&self) -> Result<u64, Error> {
        todo!()
    }
}

