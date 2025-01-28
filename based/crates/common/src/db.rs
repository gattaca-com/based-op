use alloy_primitives::Address;

pub enum DB {}

impl DB {
    pub fn get_nonce(&self, _address: Address) -> u64 {
        todo!()
    }
}
