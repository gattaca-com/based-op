use std::{io::Read, sync::Arc};

use alloy_consensus::TxEip1559;
use alloy_primitives::{map::foldhash::HashMap, ChainId, TxKind};
use bop_common::{signing::ECDSASigner, transaction::Transaction, utils::u256};
use bop_db::{BopDB, BopDbRead, Error};
use op_alloy_consensus::OpTxEnvelope;
use rand::prelude::*;
use reth_optimism_primitives::{OpBlock, OpReceipt, OpTransactionSigned};
use reth_primitives::BlockWithSenders;
use reth_provider::{
    BlockExecutionOutput, ExecutionOutcome, LatestStateProviderRef, ProviderFactory, StateWriter, TrieWriter,
};
use reth_storage_errors::provider::ProviderError;
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
            state.insert(addr1, AccountInfo::new(u256(123), 0, KECCAK_EMPTY, Bytecode::new()));
        }
        Self { state }
    }

    pub fn rand_addr(&self) -> &Address {
        let mut rng = rand::rng();

        let n = (0..self.len() - 1).choose(&mut rng).unwrap();
        self.state.keys().nth(n).unwrap()
    }

    pub fn rand_tx(&self) -> Arc<Transaction> {
        let from_addr = self.rand_addr();
        let balance = self.basic_ref(*from_addr).unwrap().unwrap().balance;
        let to_addr = self.rand_addr();
        let value = self.balance(from_addr).unwrap() / u256(10);
        let nonce = self.get_nonce(*from_addr);

        let signing_wallet = ECDSASigner::try_from_secret(from_addr.as_slice()).unwrap();
        // let chain_id =

        let tx = TxEip1559 {
            chain_id: 8400,
            nonce,
            gas_limit: 1,
            max_fee_per_gas: 1,
            max_priority_fee_per_gas: 1u128,
            to: TxKind::Call(*to_addr),
            value,
            ..Default::default()
        };

        let signed_tx = signing_wallet.sign_tx(tx).unwrap();
        Transaction::new(OpTxEnvelope::Eip1559(signed_tx), *from_addr).into()
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
    fn commit(&mut self, changes: HashMap<Address, Account>) {
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
