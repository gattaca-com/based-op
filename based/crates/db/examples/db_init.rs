use std::{collections::HashMap, str::FromStr};

use alloy_primitives::{Address, U256};
use bop_common::db::state_changes_to_bundle_state;
use bop_db::{DatabaseWrite, DatabaseRead};
use revm_primitives::{db::DatabaseRef, Account};

fn main() {
    let args = std::env::args().collect::<Vec<_>>();
    let db_dir = args[1].as_str();
    let db = bop_db::init_database(db_dir, 10_000, 100_000).expect("Failed to init DB");
    let addr = Address::from_str("0x8344fe2D13b7abCad95CFF08e4E9a070365C1309").unwrap();
    if let Some(mut info) = db.basic_ref(addr).expect("failed to query account info") {
        println!("loaded info: {info:?}");
        info.balance += U256::try_from(23).unwrap();
        info.nonce += 1;

        let account = Account::from(info);
        let changes = HashMap::from_iter(vec![(addr, account)]);
        let bundle_state = state_changes_to_bundle_state(&db, changes).unwrap();
        let (root, updates) = db.calculate_state_root(&bundle_state).unwrap();
        println!("Calculated state root: {root}, with updates {updates:?}");
    }

    // Block sync
    let _block_number = db.head_block_number().unwrap();
}
