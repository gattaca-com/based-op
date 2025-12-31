use alloy_primitives::{Address, B256, keccak256};

/// Hashes the tuple `(gateway_address, token_valid_from)` to authenticate a given portal to the gateway
pub fn gateway_auth_message(gateway: Address, valid_from: u64) -> B256 {
    let mut encoded = [0u8; 28];
    encoded[..20].copy_from_slice(gateway.as_slice());
    encoded[20..].copy_from_slice(valid_from.to_le_bytes().as_slice());

    keccak256(encoded)
}
