use std::
    sync::Arc
;

use alloy_primitives::{map::HashMap, B256};
use op_alloy_rpc_types::OpTransactionReceipt;
use parking_lot::RwLock;
use rand::RngCore;
use reth_optimism_primitives::OpBlock;
use reth_trie_common::updates::TrieUpdates;
use revm::{db::{BundleState, CacheDB,}};
use revm_primitives::{
    db::{Database, DatabaseCommit, DatabaseRef},
    Account, AccountInfo, Address, Bytecode, EvmState, U256,
};

use crate::transaction::SimulatedTx;

use super::{Error, State, state_changes_to_bundle_state, DatabaseRead};

/// DB That adds chunks on top of last on chain block
#[derive(Clone, Debug)]
pub struct DBFrag<Db> {
    pub db: Arc<RwLock<State<Db>>>,
    /// Unique identifier for the state in the db
    state_id: u64,
    // Block number for block that is currently being sorted
    curr_block_number: u64,
}

impl<Db> DBFrag<Db> {
    // pub fn reset(&mut self, db: Db) {
    //     *self.db.write() = State::(db);
    //     self.state_id = rand::rng().next_u64();
    //     self.curr_block_number += 1;
    // }
    pub fn state_id(&self) -> u64 {
        self.state_id
    }

    pub fn curr_block_number(&self) -> Result<u64, Error> {
        Ok(self.curr_block_number)
    }
}

impl<Db: DatabaseRef > DBFrag<Db> {
    pub fn commit<'a>(&mut self, txs: impl Iterator<Item = &'a SimulatedTx>) {
        let mut guard = self.db.write();

        for t in txs {
            let transitions = guard.cache.apply_evm_state(t.result_and_state.state.clone());
            if let Some(s) = guard.transition_state.as_mut() {
                s.add_transitions(transitions)
            }
        }

        self.state_id = rand::random()
    }

    pub fn commit_flat_changes(&mut self, flat_state: EvmState) {
        let mut guard = self.db.write();
        guard.commit(flat_state)
    }

    pub fn get_nonce(&self, address: Address) -> Result<u64, Error> {
        self.basic_ref(address)
            .map(|acc| acc.map(|acc| acc.nonce).unwrap_or_default())
            .map_err(|_| Error::Other("failed to get nonce".to_string()))
    }

    pub fn get_balance(&self, address: Address) -> Result<U256, Error> {
        self.basic_ref(address)
            .map(|acc| acc.map(|acc| acc.balance).unwrap_or_default())
            .map_err(|_| Error::Other("failed to get nonce".to_string()))
    }

    pub fn get_latest_block(&self) -> Result<OpBlock, Error> {
        todo!()
    }

    pub fn get_latest_block_hash(&self) -> Result<B256, Error> {
        todo!()
    }

    pub fn get_block_by_number(&self, _number: u64) -> Result<OpBlock, Error> {
        todo!()
    }

    pub fn get_block_by_hash(&self, _hash: B256) -> Result<OpBlock, Error> {
        todo!()
    }

    pub fn get_transaction_receipt(&self, _hash: B256) -> Result<OpTransactionReceipt, Error> {
        todo!()
    }

}

impl<Db: DatabaseRead> DBFrag<Db> {
    pub fn state_root(&self, state_changes: HashMap<Address, Account>) -> B256 {
        todo!();
        // let r = self.db.read();
        // let bundle_state = state_changes_to_bundle_state(&r.db, state_changes).expect("couldn't create bundle state");
        // self.calculate_state_root(&bundle_state).expect("couldn't calculate state root").0
    }
}

impl<Db: DatabaseRef> DatabaseRef for DBFrag<Db> {
    type Error = <Db as DatabaseRef>::Error;

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

impl<Db: DatabaseRef> Database for DBFrag<Db> {
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

impl<Db: DatabaseRef> DatabaseCommit for DBFrag<Db> {
    #[doc = " Commit changes to the database."]
    fn commit(&mut self,changes:HashMap<Address,Account>) {
        self.db.write().commit(changes)
    }
}

impl<Db: DatabaseRead> DatabaseRead for DBFrag<Db> {
    fn calculate_state_root(&self, bundle_state: &BundleState) -> Result<(B256, TrieUpdates), Error> {
        todo!();
        // self.db.read().calculate_state_root(bundle_state)
    }

    fn head_block_number(&self) -> Result<u64, Error> {
        Ok(self.curr_block_number - 1)
    }

    fn head_block_hash(&self) -> Result<B256, Error> {
        todo!();
        // self.db.read().head_block_hash()
    }
}

impl<Db: DatabaseRead + Database> From<Db> for DBFrag<Db> {
    fn from(value: Db) -> Self {
        let curr_block_number = value.head_block_number().unwrap() + 1;
        let state = State::builder().with_database(value).with_bundle_update().without_state_clear().build();
        Self { db: Arc::new(RwLock::new(state)), state_id: rand::random(), curr_block_number }
    }
}

