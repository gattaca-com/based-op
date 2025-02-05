use std::sync::Arc;

use alloy_consensus::{transaction::Transaction as TransactionTrait, Header};
use bop_common::{
    actor::{Actor, ActorConfig},
    communication::{
        messages::{EvmBlockParams, SequencerToSimulator, SimulationError, SimulatorToSequencer, SimulatorToSequencerMsg},
        SpineConnections, TrackedSenders,
    },
    db::{DBFrag, DBSorting, DatabaseRead},
    time::Duration,
    transaction::{SimulatedTx, Transaction},
};
use reth_evm::{env::EvmEnv, system_calls::SystemCaller, ConfigureEvm, ConfigureEvmEnv, NextBlockEnvAttributes};
use reth_optimism_chainspec::{OpChainSpec, OpChainSpecBuilder};
use reth_optimism_evm::OpEvmConfig;
use revm::{db::CacheDB, DatabaseRef, Evm};
use revm_primitives::{BlockEnv, EnvWithHandlerCfg, SpecId};
use tracing::{info, instrument::WithSubscriber};

/// Simulator thread.
/// 
/// TODO: need to impl fn to use system caller and return changes for that.
pub struct Simulator<'a, Db: DatabaseRef> {
    /// Top of frag evm.
    evm_tof: Evm<'a, (), CacheDB<DBFrag<Db>>>,

    /// Evm on top of partially built frag
    evm_sorting: Evm<'a, (), CacheDB<Arc<DBSorting<Db>>>>,
    
    /// Utility to call system smart contracts.
    system_caller: SystemCaller<OpEvmConfig, OpChainSpec>,
    /// How to create an EVM.
    evm_config: OpEvmConfig,
}

impl<'a, Db: DatabaseRead> Simulator<'a, Db> {
    pub fn new(db: DBFrag<Db>, evm_config: &'a OpEvmConfig) -> Self {
        let system_caller = SystemCaller::new(evm_config.clone(), evm_config.chain_spec().clone());

        // Initialise with default evms. These will be overridden before the first sim by
        // `set_evm_for_new_block`.
        let db_tof = CacheDB::new(db.clone());
        let evm_tof: Evm<'_, (), _> = evm_config.evm(db_tof);
        let db_sorting = CacheDB::new(Arc::new(DBSorting::new(db)));
        let evm_sorting: Evm<'_, (), _> = evm_config.evm(db_sorting);

        Self { evm_sorting, evm_tof, system_caller, evm_config: evm_config.clone() }
    }

    fn simulate_tx<DbRef: DatabaseRead>(
        tx: Arc<Transaction>,
        db: DbRef,
        evm: &mut Evm<'a, (), CacheDB<DbRef>>,
    ) -> Result<SimulatedTx, SimulationError<<DbRef as DatabaseRef>::Error>> {
        tracing::debug!("simming tx for {:?}", tx.chain_id());
        let old_db = std::mem::replace(evm.db_mut(),CacheDB::new(db));
        tx.fill_tx_env(evm.tx_mut());
        tracing::debug!("simming tx on evm {:?}", evm.context.evm.env.cfg.chain_id);
        let res = evm.transact()?;
        let simtx = SimulatedTx::new(tx, res, evm.db(), evm.block().coinbase);
        // This dance is needed to drop the arc ref
        let _ = std::mem::replace(evm.db_mut(), old_db);
        Ok(simtx)
    }

    fn set_evm_for_new_block(&mut self, evm_block_params: EvmBlockParams) {
        let parent = &evm_block_params.header;
        let next_attributes = evm_block_params.attributes;

        // Initialise evm cfg and block env for the next block.
        let evm_env = self.evm_config.next_cfg_and_block_env(parent, next_attributes).unwrap();
        let EvmEnv { cfg_env_with_handler_cfg, block_env } = evm_env;
        let env_with_handler_cfg = EnvWithHandlerCfg::new_with_cfg_env(cfg_env_with_handler_cfg, block_env, Default::default());

        // Update evms with the new env.
        self.evm_tof.modify_spec_id(env_with_handler_cfg.spec_id());
        self.evm_tof.context.evm.env = env_with_handler_cfg.env.clone();

        self.evm_sorting.modify_spec_id(env_with_handler_cfg.spec_id());
        self.evm_sorting.context.evm.env = env_with_handler_cfg.env;
    }
}

impl<'a, Db: DatabaseRead> Actor<Db> for Simulator<'a, Db> {
    const CORE_AFFINITY: Option<usize> = None;

    fn loop_body(&mut self, connections: &mut SpineConnections<Db>) {
        // Received each new block from the sequencer.
        connections.receive(|msg: EvmBlockParams, _| {
            self.set_evm_for_new_block(msg);
        });

        connections.receive(|msg: SequencerToSimulator<Db>, senders| {
            match msg {
                // TODO: Cleanup: merge both functions?
                SequencerToSimulator::SimulateTx(tx, db) => {
                    info!("simulating at state_id {}", db.state_id());
                    let _ = senders.send_timeout(
                        SimulatorToSequencer::new(
                            (tx.sender(), tx.nonce()),
                            db.state_id(),
                            SimulatorToSequencerMsg::Tx(Self::simulate_tx(tx, db, &mut self.evm_sorting)),
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
    }
}
