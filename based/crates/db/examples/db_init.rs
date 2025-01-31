use std::{collections::HashMap, str::FromStr};

use alloy_primitives::{Address, U256};
use bop_db::{BopDB, BopDbRead};
use revm_primitives::{db::DatabaseRef, Account};

fn main() {
    let args = std::env::args().collect::<Vec<_>>();
    let db_dir = args[1].as_str();
    let db = bop_db::init_database(db_dir, 10_000, 100_000).expect("Failed to init DB");
    let db_ro = db.readonly().expect("failed to get readonly DB");
    let addr = Address::from_str("0x8344fe2D13b7abCad95CFF08e4E9a070365C1309").unwrap();
    if let Some(mut info) = db_ro.basic_ref(addr).expect("failed to query account info") {
        println!("loaded info: {info:?}");
        info.balance = info.balance + U256::try_from(23).unwrap();
        info.nonce += 1;

        let account = Account::from(info);
        let changes = HashMap::from_iter(vec![(addr, account)]);
        let bundle_state = bop_db::state_changes_to_bundle_state(&db_ro, changes).unwrap();
        let (root, updates) = db_ro.calculate_state_root(&bundle_state).unwrap();
        println!("Calculated state root: {root}, with updates {updates:?}");
    }
}
