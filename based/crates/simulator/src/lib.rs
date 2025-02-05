use std::sync::Arc;

use alloy_consensus::transaction::Transaction as TransactionTrait;
use bop_common::{
    actor::{Actor, ActorConfig},
    communication::{
        messages::{SequencerToSimulator, SimulationError, SimulatorToSequencer, SimulatorToSequencerMsg},
        SpineConnections, TrackedSenders,
    },
    db::{DBFrag, DBSorting, DatabaseRead},
    time::Duration,
    transaction::{SimulatedTx, Transaction},
};
use reth_evm::ConfigureEvm;
use reth_optimism_chainspec::OpChainSpecBuilder;
use reth_optimism_evm::OpEvmConfig;
use revm::{db::CacheDB, DatabaseRef, Evm};
use revm_primitives::{BlockEnv, SpecId};
use tracing::{info, instrument::WithSubscriber};
use alloy_consensus::transaction::Transaction as TransactionTrait;

pub struct Simulator<'a, Db: DatabaseRef> {
    /// Top of frag evm
    evm_tof: Evm<'a, (), CacheDB<DBFrag<Db>>>,
    /// Evm on top of partially built frag
    evm: Evm<'a, (), CacheDB<Arc<DBSorting<Db>>>>,
}

impl<'a, Db: DatabaseRead> Simulator<'a, Db> {
    pub fn create_and_run(connections: SpineConnections<Db>, db: DBFrag<Db>, actor_config: ActorConfig, evmconfig: OpEvmConfig) {
        
        tracing::error!("{:?}", evmconfig.chain_spec().chain());
        let cache_tof = CacheDB::new(db.clone());
        let mut evm_tof: Evm<'_, (), _> = evmconfig.evm(cache_tof);
        evm_tof.context.evm.env.cfg.chain_id = evmconfig.chain_spec().chain.id();
        let cache = CacheDB::new(Arc::new(DBSorting::new(db)));
        let mut evm: Evm<'_, (), _> = evmconfig.evm(cache);
        evm.context.evm.env.cfg.chain_id = evmconfig.chain_spec().chain.id();
        Simulator::new(evm_tof, evm).run(connections, actor_config);
    }

    pub fn new(evm_tof: Evm<'a, (), CacheDB<DBFrag<Db>>>, evm: Evm<'a, (), CacheDB<Arc<DBSorting<Db>>>>) -> Self {
        Self { evm, evm_tof }
    }

    fn simulate_tx<DbRef: DatabaseRead>(
        tx: Arc<Transaction>,
        db: DbRef,
        evm: &mut Evm<'a, (), CacheDB<DbRef>>,
    ) -> Result<SimulatedTx, SimulationError<<DbRef as DatabaseRef>::Error>> {
        info!("simming tx for {:?}", tx.chain_id());
        let old_db = std::mem::replace(evm.db_mut(),CacheDB::new(db));
        tx.fill_tx_env(evm.tx_mut());
        info!("simming tx on evm {:?}", evm.context.evm.env.cfg.chain_id);
        let res = evm.transact()?;
        let simtx = SimulatedTx::new(tx, res, evm.db(), evm.block().coinbase);
        // This dance is needed to drop the arc ref
        let _ = std::mem::replace(evm.db_mut(), old_db);
        Ok(simtx)
    }

    fn set_blockenv(&mut self, env: BlockEnv) {
        *self.evm.block_mut() = env.clone();
        *self.evm_tof.block_mut() = env;
    }

    #[allow(dead_code)]
    fn set_spec_id(&mut self, spec_id: SpecId) {
        self.evm.modify_spec_id(spec_id);
    }
}

impl<Db: DatabaseRead> Actor<Db> for Simulator<'_, Db> {
    const CORE_AFFINITY: Option<usize> = None;

    fn loop_body(&mut self, connections: &mut SpineConnections<Db>) {
        connections.receive(|msg: SequencerToSimulator<Db>, senders| {
            match msg {
                // TODO: Cleanup: merge both functions?
                SequencerToSimulator::SimulateTx(tx, db) => {
                    info!("simulating at state_id {}", db.state_id());
                    let _ = senders.send_timeout(
                        SimulatorToSequencer::new(
                            (tx.sender(), tx.nonce()),
                            db.state_id(),
                            SimulatorToSequencerMsg::Tx(Self::simulate_tx(tx, db, &mut self.evm)),
                        ),
                        Duration::from_millis(10),
                    );
                }
                SequencerToSimulator::SimulateTxTof(tx, db) => {
                    info!("simulating at state_id {}", db.state_id());
                    let _ = senders.send_timeout(
                        SimulatorToSequencer::new(
                            (tx.sender(), tx.nonce()),
                            db.state_id(),
                            SimulatorToSequencerMsg::TxPoolTopOfFrag(Self::simulate_tx(tx, db, &mut self.evm_tof)),
                        ),
                        Duration::from_millis(10),
                    );
                }
            }
        });
        connections.receive(|msg, _| {
            self.set_blockenv(msg);
        });
    }
}
