use std::{mem::MaybeUninit, sync::Arc};

use bop_common::{
    actor::{Actor, ActorConfig},
    communication::{
        messages::{SequencerToSimulator, SimulationError},
        Connections, ReceiversSpine, SendersSpine, Spine, SpineConnections, TrackedSenders,
    },
    db::{BopDB, BopDbRead, DBFrag, DBSorting},
    time::Duration,
    transaction::{SimulatedTx, Transaction},
    utils::last_part_of_typename,
};
use reth_evm::ConfigureEvm;
use reth_optimism_chainspec::{OpChainSpec, OpChainSpecBuilder};
use reth_optimism_evm::OpEvmConfig;
use revm::{db::CacheDB, DatabaseRef, Evm};
use revm_primitives::{BlockEnv, SpecId};
use tracing::info;

pub struct Simulator<'a, Db: DatabaseRef> {
    /// Top of frag evm
    evm_tof: Evm<'a, (), CacheDB<Db>>,
    /// Evm on top of partially built frag
    evm: Evm<'a, (), CacheDB<Arc<CacheDB<Db>>>>,
}

impl<'a, Db: BopDbRead> Simulator<'a, Db> {
    pub fn create_and_run(connections: SpineConnections<Db>, db: DBFrag<Db>, actor_config: ActorConfig) {
        let chainspec = Arc::new(OpChainSpecBuilder::base_mainnet().build());
        let evmconfig = OpEvmConfig::new(chainspec);

        let cache_tof = CacheDB::new(db.clone());
        let evm_tof: Evm<'_, (), _> = evmconfig.evm(cache_tof);
        let cache = CacheDB::new(Arc::new(CacheDB::new(db)));
        let evm: Evm<'_, (), _> = evmconfig.evm(cache);
        Simulator::new(evm_tof, evm).run(connections, actor_config);
    }

    pub fn new(evm_tof: Evm<'a, (), CacheDB<Db>>, evm: Evm<'a, (), CacheDB<Arc<CacheDB<Db>>>>) -> Self {
        Self { evm, evm_tof }
    }

    fn simulate_tx(
        &mut self,
        tx: Arc<Transaction>,
    ) -> Result<SimulatedTx, SimulationError<<Db as DatabaseRef>::Error>> {
        tx.fill_tx_env(self.evm.tx_mut());
        let res = self.evm.transact()?;
        Ok(SimulatedTx::new(tx, res, self.evm.db(), self.evm.block().coinbase))
    }

    fn set_blockenv(&mut self, env: BlockEnv) {
        *self.evm.block_mut() = env;
    }

    fn set_db(&mut self, db: Arc<CacheDB<Db>>) {
        *self.evm.db_mut() = CacheDB::new(db);
    }

    fn set_spec_id(&mut self, spec_id: SpecId) {
        self.evm.modify_spec_id(spec_id);
    }
}

impl<Db: BopDbRead> Actor<Db> for Simulator<'_, DBFrag<Db>> {
    const CORE_AFFINITY: Option<usize> = None;

    fn loop_body(&mut self, connections: &mut SpineConnections<Db>) {
        connections.receive(|msg: SequencerToSimulator<Db>, senders| {
            info!("received {}", msg.as_ref());
            match msg {
                SequencerToSimulator::SimulateTx(_db, tx) => {
                    todo!()
                }
                SequencerToSimulator::SimulateTxTof(_db, tx) => {
                    todo!()
                }
                SequencerToSimulator::NewBlock => {
                    todo!()
                }
            }
        });
    }
}
