// use bop_db::BopDB;
// use builder_common::inspectors::AddressScreener;
// use builder_evm::db::{CacheDBRwLock, DB};
// use revm::{
//     db::{CacheDB, Database},
//     Evm,
// };

// use super::Simulator;

// /// Evms is a struct that holds multiple EVM instances for each type of DB and screening status, this allows to use
// the /// same EVM instances across simulations
// pub struct Evms<'a, Db: BopDB> {
//     pub tob: Evm<'a, AddressScreener, CacheDB<Db>>,
//     pub partially_built: Evm<'a, AddressScreener, CacheDB<Arc<CacheDB<Db>>>>,
// }

// impl<'a> Evms<'a> {
//     pub fn new(simulator: &Simulator) -> Self {
//         // these DBs are never actually used in practice as they are swapped before each sim

//         let cache = simulator.build_evm_owned(simulator.state(), false, false);
//         let rw = simulator.build_evm_owned(CacheDBRwLock::new(DB::Empty), false, false);
//         let rw_cache = simulator.build_evm_owned(CacheDB::new(CacheDBRwLock::new(DB::Empty)), false, false);

//         Self { cache, rw, rw_cache }
//     }

//     /// This is potentially slow , should be called at most once per block
//     pub fn reset_evms(&mut self, simulator: &Simulator) {
//         self.cache = simulator.build_evm_owned(simulator.state(), false, false);
//         self.rw = simulator.build_evm_owned(CacheDBRwLock::new(DB::Empty), false, false);
//         self.rw_cache = simulator.build_evm_owned(CacheDB::new(CacheDBRwLock::new(DB::Empty)), false, false);
//     }
// }

// pub fn prepare_evm_for_sim<DB: Database>(
//     evm: &mut Evm<'_, AddressScreener, DB>,
//     db: DB,
//     needs_screening: bool,
//     build_access_list: bool,
// ) {
//     evm.context.evm.db = db;
//     evm.context.external.prepare_for_sim(needs_screening, build_access_list);
// }
