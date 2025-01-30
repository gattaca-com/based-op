use std::fmt::Debug;

use alloy_primitives::Address;
use revm::{DatabaseCommit, DatabaseRef};

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
