use std::sync::{Arc, RwLock};

use alloy_primitives::{Address, B256, U256};
use revm::db::{AccountState, CacheDB, DbAccount};
use revm_primitives::{
    db::{DatabaseCommit, DatabaseRef},
    Account, AccountInfo, Bytecode, HashMap, KECCAK_EMPTY,
};

use crate::BopDB;
