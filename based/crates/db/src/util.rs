use alloy_primitives::{map::HashMap, Address};
use revm::db::BundleState;
use revm_primitives::{db::DatabaseRef, keccak256, Account, B256, U256};

/// Converts the index (slot) passed from the EVM in the `storage_ref` call
/// to the hash used as the DB storage key.
pub fn index_to_storage_key(index: &U256) -> B256 {
    let bytes: [u8; 32] = index.to_be_bytes();
    keccak256(bytes)
}
