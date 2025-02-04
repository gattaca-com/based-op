use std::sync::Arc;

use bop_common::{
    actor::{Actor, ActorConfig},
    communication::{
        messages::{SequencerToSimulator, SimulationError, SimulatorToSequencer, SimulatorToSequencerMsg},
        SpineConnections, TrackedSenders,
    },
    db::{BopDbRead, DBFrag, DBSorting},
    time::Duration,
    transaction::{SimulatedTx, Transaction},
};
use reth_evm::ConfigureEvm;
use reth_optimism_chainspec::OpChainSpecBuilder;
use reth_optimism_evm::OpEvmConfig;
use revm::{db::CacheDB, DatabaseRef, Evm};
use revm_primitives::{BlockEnv, SpecId};
use tracing::info;

pub struct Simulator<'a, Db: DatabaseRef> {
    /// Top of frag evm
    evm_tof: Evm<'a, (), CacheDB<DBFrag<Db>>>,
    /// Evm on top of partially built frag
    evm: Evm<'a, (), CacheDB<Arc<DBSorting<Db>>>>,
}

impl<'a, Db: BopDbRead> Simulator<'a, Db> {
    pub fn create_and_run(connections: SpineConnections<Db>, db: DBFrag<Db>, actor_config: ActorConfig) {
        //TODO: Is this fine?
        let chainspec = Arc::new(OpChainSpecBuilder::base_mainnet().build());
        let evmconfig = OpEvmConfig::new(chainspec);

        let cache_tof = CacheDB::new(db.clone());
        let evm_tof: Evm<'_, (), _> = evmconfig.evm(cache_tof);
        let cache = CacheDB::new(Arc::new(DBSorting::new(db)));
        let evm: Evm<'_, (), _> = evmconfig.evm(cache);
        Simulator::new(evm_tof, evm).run(connections, actor_config);
    }

    pub fn new(evm_tof: Evm<'a, (), CacheDB<DBFrag<Db>>>, evm: Evm<'a, (), CacheDB<Arc<DBSorting<Db>>>>) -> Self {
        Self { evm, evm_tof }
    }

    fn simulate_tx<DbRef: BopDbRead>(
        tx: Arc<Transaction>,
        db: DbRef,
        evm: &mut Evm<'a, (), CacheDB<DbRef>>,
    ) -> Result<SimulatedTx, SimulationError<<DbRef as DatabaseRef>::Error>> {
        *evm.db_mut() = CacheDB::new(db);
        tx.fill_tx_env(evm.tx_mut());
        let res = evm.transact()?;
        let simtx = SimulatedTx::new(tx, res, evm.db(), evm.block().coinbase);
        Ok(simtx)
    }

    fn set_blockenv(&mut self, env: BlockEnv) {
        *self.evm.block_mut() = env.clone();
        *self.evm_tof.block_mut() = env;
    }

    fn set_spec_id(&mut self, spec_id: SpecId) {
        self.evm.modify_spec_id(spec_id);
    }
}

impl<Db: BopDbRead> Actor<Db> for Simulator<'_, Db> {
    const CORE_AFFINITY: Option<usize> = None;

    fn loop_body(&mut self, connections: &mut SpineConnections<Db>) {
        connections.receive(|msg: SequencerToSimulator<Db>, senders| {
            info!("received {}", msg.as_ref());
            match msg {
                // TODO: Cleanup: merge both functions?
                SequencerToSimulator::SimulateTx(tx, db) => {
                    let sender = tx.sender();
                    let _ = senders.send_timeout(
                        SimulatorToSequencer::new(
                            sender,
                            db.unique_hash(),
                            SimulatorToSequencerMsg::Tx(Self::simulate_tx(tx, db, &mut self.evm)),
                        ),
                        Duration::from_millis(10),
                    );
                }
                SequencerToSimulator::SimulateTxTof(tx, db) => {
                    let sender = tx.sender();
                    let _ = senders.send_timeout(
                        SimulatorToSequencer::new(
                            sender,
                            db.unique_hash(),
                            SimulatorToSequencerMsg::TxTof(Self::simulate_tx(tx, db, &mut self.evm_tof)),
                        ),
                        Duration::from_millis(10),
                    );
                }
                SequencerToSimulator::NewBlock => {
                    todo!()
                }
            }
        });
        connections.receive(|msg, _| {
            self.set_blockenv(msg);
        });
    }
}
