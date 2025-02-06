use std::sync::Arc;

use alloy_consensus::{transaction::Transaction as TransactionTrait, Header};
use bop_common::{
    actor::{Actor, ActorConfig},
    communication::{
        messages::{
            EvmBlockParams, NextBlockAttributes, SequencerToSimulator, SimulationError, SimulatorToSequencer, SimulatorToSequencerMsg
        },
        SpineConnections, TrackedSenders,
    },
    db::{DBFrag, DBSorting, DatabaseRead},
    time::Duration,
    transaction::{SimulatedTx, Transaction},
};
use reth_evm::{env::EvmEnv, system_calls::SystemCaller, ConfigureEvm, ConfigureEvmEnv, NextBlockEnvAttributes};
use reth_optimism_chainspec::{OpChainSpec, OpChainSpecBuilder};
use reth_optimism_evm::{ensure_create2_deployer, OpBlockExecutionError, OpEvmConfig};
use revm::{db::CacheDB, Database, DatabaseRef, Evm, State};
use revm_primitives::{BlockEnv, EnvWithHandlerCfg, SpecId};
use tracing::{info, instrument::WithSubscriber};
use reth_chainspec::EthereumHardforks;
use reth_optimism_forks::OpHardfork;

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

    /// Simulate all txs in the forced inclusion txs from PayloadAttributes
    /// and any pre-execution changes.
    fn simulate_forced_inclusion_txs<DbRef: DatabaseRead>(
        txs: Vec<Arc<Transaction>>,
        db: DbRef,
        evm: &mut Evm<'a, (), CacheDB<DbRef>>,
    ) -> Result<SimulatedTx, SimulationError<<DbRef as DatabaseRef>::Error>> {
        let old_db = std::mem::replace(evm.db_mut(), CacheDB::new(db));


    }

    fn simulate_tx<DbRef: DatabaseRead>(
        tx: Arc<Transaction>,
        db: DbRef,
        evm: &mut Evm<'a, (), CacheDB<DbRef>>,
    ) -> Result<SimulatedTx, SimulationError<<DbRef as DatabaseRef>::Error>> {
        let old_db = std::mem::replace(evm.db_mut(), CacheDB::new(db));

        tx.fill_tx_env(evm.tx_mut());
        let res = evm.transact()?;

        // This dance is needed to drop the arc ref
        let _ = std::mem::replace(evm.db_mut(), old_db);

        Ok(SimulatedTx::new(tx, res, evm.db(), evm.block().coinbase))
    }

    /// Called each new block from the sequencer.
    /// 1) Updates both evms with the new env.
    /// 2) Applies pre-execution changes.
    /// 3) Applies any forced inclusion txs.
    /// 4) Returns the end state and SimulatedTxs.
    /// 
    fn on_new_block(&mut self, evm_block_params: EvmBlockParams) -> Result<(), ()> {  // TODO: error
        // Update both evms with the new env.
        self.set_env_for_new_block(evm_block_params);

        // Get start state for sorting.
        self.apply_pre_execution_changes(&evm_block_params.&mut self.evm_sorting)?;
    }
    
    fn set_env_for_new_block(&mut self, evm_block_params: &EvmBlockParams) {
        let parent = &evm_block_params.parent_header;
        let next_attributes = evm_block_params.attributes.clone();

        // Initialise evm cfg and block env for the next block.
        let evm_env = self.evm_config.next_cfg_and_block_env(parent, next_attributes).unwrap();
        let EvmEnv { cfg_env_with_handler_cfg, block_env } = evm_env;
        let env_with_handler_cfg =
            EnvWithHandlerCfg::new_with_cfg_env(cfg_env_with_handler_cfg, block_env, Default::default());

        // Update evms with the new env.
        self.evm_tof.modify_spec_id(env_with_handler_cfg.spec_id());
        self.evm_tof.context.evm.env = env_with_handler_cfg.env.clone();

        self.evm_sorting.modify_spec_id(env_with_handler_cfg.spec_id());
        self.evm_sorting.context.evm.env = env_with_handler_cfg.env;
    }

    fn apply_pre_execution_changes(
        &self,
        next_attributes: &NextBlockAttributes,
        evm: &mut Evm<'a, (), impl Database>,
        db: CacheDB<impl DatabaseRef>
    ) -> Result<(), ()> {  // TODO: error
        let mut state =
            State::builder().with_database(db).with_bundle_update().without_state_clear().build();
        let old_db = std::mem::replace(evm.db_mut(), state);

        let chain_spec = self.evm_config.chain_spec().clone();
        let block_number = u64::try_from(evm.block().number).unwrap();
        let block_timestamp = u64::try_from(evm.block().timestamp).unwrap();

        // Set state clear flag if the block is after the Spurious Dragon hardfork.
        let state_clear_flag = chain_spec.is_spurious_dragon_active_at_block(block_number);
        evm.db_mut().set_state_clear_flag(state_clear_flag);

        self.system_caller.apply_beacon_root_contract_call(
            block_timestamp,
            block_number,
            next_attributes.parent_beacon_block_root,
            &mut state,
        )?;

        // Ensure that the create2deployer is force-deployed at the canyon transition. Optimism
        // blocks will always have at least a single transaction in them (the L1 info transaction),
        // so we can safely assume that this will always be triggered upon the transition and that
        // the above check for empty blocks will never be hit on OP chains.
        ensure_create2_deployer(chain_spec, block_timestamp, evm.db_mut())
            .map_err(|_| OpBlockExecutionError::ForceCreate2DeployerFail).unwrap();  // TODO: error

        Ok(())
    }
}

impl<'a, Db: DatabaseRead> Actor<Db> for Simulator<'a, Db> {
    const CORE_AFFINITY: Option<usize> = None;

    fn loop_body(&mut self, connections: &mut SpineConnections<Db>) {
        // Received each new block from the sequencer.
        connections.receive(|msg: EvmBlockParams, _| {
            self.set_env_for_new_block(msg);
        });

        connections.receive(|msg: SequencerToSimulator<Db>, senders| {
            match msg {
                // TODO: Cleanup: merge both functions?
                SequencerToSimulator::SimulateTx(tx, db) => {
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
