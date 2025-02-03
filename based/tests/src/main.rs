#[cfg(not(test))]
fn main() {}

#[cfg(test)]
mod db;
#[cfg(test)]
mod bop_tests {
    use std::sync::Arc;

    use alloy_primitives::{
        map::foldhash::{HashMap, HashMapExt},
        Address, U256,
    };
    use bop
    use bop_db::BopDB;
    use bop_sequencer::SequencerConfig;
    use revm::{
        primitives::{AccountInfo, Bytecode, KECCAK_EMPTY},
        DatabaseRef,
    };

    #[test]
    fn db() {
        let db = super::db::GenesisDB::new_with_randoms(10);
        assert_eq!(db.len(), 10);
        assert_eq!(db.balance(db.get_rand_addr()), Some(U256::from_limbs([123, 0, 0, 0])));
    }

    #[test]
    fn sequencer() {
        let db_bop = super::db::GenesisDB::new_with_randoms(10);
        let spine = bop_common::communication::Spine::default();
        let db_read: bop_common::db::DBFrag<_> = db_bop.readonly().expect("Failed to create read-only DB").into();
        std::thread::scope(|s| {
            let rt = tokio::runtime::Builder::new_current_thread()
                .worker_threads(1)
                .enable_all()
                .build()
                .expect("failed to create runtime")
                .into();
            let sequencer = bop_sequencer::Sequencer::new(db_bop, db_read, rt, SequencerConfig::default());
            sequencer.run(
        })
    }
}
