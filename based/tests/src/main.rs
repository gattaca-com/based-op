#[cfg(not(test))]
fn main() {}

#[cfg(test)]
mod db;
#[cfg(test)]
mod bop_tests {
    use alloy_primitives::{
        map::foldhash::{HashMap, HashMapExt},
        Address, U256,
    };
    use revm::{
        primitives::{AccountInfo, Bytecode, KECCAK_EMPTY},
        DatabaseRef,
    };

    #[test]
    fn db() {
        let mut state = HashMap::new();
        let addr1 = Address::random();
        state.insert(addr1, AccountInfo::new(U256::from_limbs([123, 0, 0, 0]), 0, KECCAK_EMPTY, Bytecode::new()));

        let db = super::db::GenesisDB::new(state);
        assert_eq!(db.basic_ref(addr1).unwrap().map(|t| t.balance), Some(U256::from_limbs([123, 0, 0, 0])));
    }
}
