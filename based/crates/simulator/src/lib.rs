use std::{mem::MaybeUninit, sync::Arc};

use bop_common::{
    actor::{Actor, ActorConfig},
    communication::{
        messages::{SequencerToSimulator, SimulationError},
        Connections, ReceiversSpine, SendersSpine, Spine, SpineConnections, TrackedSenders,
    },
    time::Duration,
    transaction::{SimulatedTx, Transaction},
    utils::last_part_of_typename,
};
use bop_db::{BopDB, BopDbRead, DBSorting};
use reth_evm::ConfigureEvm;
use reth_optimism_chainspec::{OpChainSpec, OpChainSpecBuilder};
use reth_optimism_evm::OpEvmConfig;
use revm::{db::CacheDB, DatabaseRef, Evm};
use revm_primitives::{BlockEnv, SpecId};
use tracing::info;

pub struct Simulator<'a, Db: DatabaseRef> {
    id: usize,
    evm: Evm<'a, (), CacheDB<DBSorting<Db>>>,
}

impl<'a, Db: BopDbRead> Simulator<'a, Db> {
    pub fn create_and_run(connections: SpineConnections<Db>, db: Db, id: usize, actor_config: ActorConfig) {
        let chainspec = Arc::new(OpChainSpecBuilder::base_mainnet().build());
        let evmconfig = OpEvmConfig::new(chainspec);
        let cache = CacheDB::new(Arc::new(CacheDB::new(Arc::new(CacheDB::new(db)))));
        let evm: Evm<'_, (), _> = evmconfig.evm(cache);
        Simulator::new(id, evm).run(connections, actor_config);
    }

    pub fn new(id: usize, evm: Evm<'a, (), DBSorting<Db>>) -> Self {
        Self { id, evm }
    }

    fn simulate_tx(&mut self, tx: Arc<Transaction>) -> Result<SimulatedTx, SimulationError> {
        todo!()
        // tx.fill_tx_env(self.evm.tx_mut());
        // let res = self.evm.transact()
        // SimulatedTx {
        //     tx,

        // }
    }

    fn set_blockenv(&mut self, env: BlockEnv) {
        *self.evm.block_mut() = env;
    }

    fn set_db(&mut self, db: DBSorting<Db>) {
        *self.evm.db_mut() = db;
    }

    fn set_spec_id(&mut self, spec_id: SpecId) {
        self.evm.modify_spec_id(spec_id);
    }
}

impl<Db: BopDbRead> Actor<Db> for Simulator<'_, Db> {
    const CORE_AFFINITY: Option<usize> = None;

    fn loop_body(&mut self, connections: &mut Connections<SendersSpine<Db>, ReceiversSpine<Db>>) {
        connections.receive(|msg: SequencerToSimulator<Db>, senders| {
            info!("received {}", msg.as_ref());
            match msg {
                SequencerToSimulator::SimulateTx(_db, tx) => {
                    todo!()
                }
                SequencerToSimulator::NewBlock => {
                    todo!()
                }
            }
        });
    }
}
