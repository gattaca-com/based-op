use std::{
    fmt::{Debug, Display},
    sync::Arc,
};

use alloy_consensus::{transaction::Transaction as TransactionTrait, Header};
use bop_common::{
    actor::{Actor, ActorConfig},
    communication::{
        messages::{
            EvmBlockParams, NextBlockAttributes, SequencerToSimulator, SimulationError, SimulatorToSequencer,
            SimulatorToSequencerMsg, TopOfBlockResult,
        },
        SendersSpine, SpineConnections, TrackedSenders,
    },
    db::{DBFrag, DBSorting},
    time::Duration,
    transaction::{SimulatedTx, Transaction},
};
use reth_chainspec::EthereumHardforks;
use reth_evm::{
    env::EvmEnv,
    execute::{BlockExecutionError, BlockValidationError, ProviderError},
    system_calls::SystemCaller,
    ConfigureEvm, ConfigureEvmEnv,
};
use reth_optimism_chainspec::OpChainSpec;
use reth_optimism_evm::{ensure_create2_deployer, OpBlockExecutionError, OpEvmConfig};
use revm::{
    db::{CacheDB, State},
    CacheState, Database, DatabaseCommit, DatabaseRef, Evm,
};
use revm_primitives::{Address, BlockEnv, EnvWithHandlerCfg, ResultAndState, SpecId};
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
    id: usize,
}

impl<'a, Db: DatabaseRef + Clone> Simulator<'a, Db>
where
    <Db as DatabaseRef>::Error: Into<ProviderError> + Debug + Display,
{
    pub fn new(db: DBFrag<Db>, evm_config: &'a OpEvmConfig, id: usize) -> Self {
        let system_caller = SystemCaller::new(evm_config.clone(), evm_config.chain_spec().clone());

        // Initialise with default evms. These will be overridden before the first sim by
        // `set_evm_for_new_block`.
        let db_tof = CacheDB::new(db.clone());
        let evm_tof: Evm<'_, (), _> = evm_config.evm(db_tof);
        let db_sorting = CacheDB::new(Arc::new(DBSorting::new(db)));
        let evm_sorting: Evm<'_, (), _> = evm_config.evm(db_sorting);

        Self { evm_sorting, evm_tof, system_caller, evm_config: evm_config.clone(), id }
    }

    /// finalise
    fn simulate_tx<SimulateTxDb: DatabaseRef>(
        tx: Arc<Transaction>,
        db: SimulateTxDb,
        evm: &mut Evm<'a, (), CacheDB<SimulateTxDb>>,
    ) -> Result<SimulatedTx, SimulationError> {
        let coinbase = evm.block().coinbase;
        let start_balance = evm.db_mut().basic(coinbase).ok().flatten().map(|a| a.balance).unwrap_or_default();

        let old_db = std::mem::replace(evm.db_mut(), CacheDB::new(db));
        tx.fill_tx_env(evm.tx_mut());
        let res = evm.transact().map_err(|_e| SimulationError::EvmError("todo 2".to_string()))?;

        // This dance is needed to drop the arc ref
        let _ = std::mem::replace(evm.db_mut(), old_db);

        Ok(SimulatedTx::new(tx, res, start_balance, coinbase))
    }

    /// Processes a new block from the sequencer by:
    /// 1. Updating EVM environments
    /// 2. Applying pre-execution changes
    /// 3. Processing forced inclusion transactions
    fn on_new_block(&mut self, evm_block_params: EvmBlockParams<Db>, senders: &SendersSpine<Db>) {
        let env_with_handler_cfg = self.get_env_for_new_block(&evm_block_params);
        self.update_evm_environments(&env_with_handler_cfg);
        let (forced_inclusion_txs, cache_state) = self
            .get_start_state_for_new_block(evm_block_params.db, env_with_handler_cfg, &evm_block_params.attributes)
            .expect("shouldn't fail");
        let msg = SimulatorToSequencer::new(
            (Address::random(), 0),
            0,
            SimulatorToSequencerMsg::TopOfBlock(TopOfBlockResult { cache_state, forced_inclusion_txs }),
        );
        senders.send_forever(msg);
    }

    /// Must be called each new block.
    /// Applies pre-execution changes and must include txs from the payload attributes.
    ///
    /// Returns the end state and SimulatedTxs for all must include txs.
    fn get_start_state_for_new_block(
        &mut self,
        db: DBFrag<Db>,
        env_with_handler_cfg: EnvWithHandlerCfg,
        next_attributes: &NextBlockAttributes,
    ) -> Result<(Vec<SimulatedTx>, CacheState), BlockExecutionError> {
        let evm_config = self.evm_config.clone();

        // Configure new EVM to apply pre-execution and must include txs.
        let mut state = State::builder().with_database(db).with_bundle_update().without_state_clear().build();
        let mut evm = evm_config.evm_with_env(&mut state, env_with_handler_cfg);

        // Apply pre-execution changes.
        self.apply_pre_execution_changes(next_attributes, &mut evm)?;

        let mut tx_results = Vec::with_capacity(next_attributes.forced_inclusion_txs.len());
        let block_coinbase = evm.block().coinbase;
        // Apply must include txs.
        for tx in next_attributes.forced_inclusion_txs.iter() {
            let start_balance =
                evm.db_mut().basic(block_coinbase).ok().flatten().map(|a| a.balance).unwrap_or_default();

            tx.fill_tx_env(evm.tx_mut());

            // Execute transaction.
            let result_and_state = evm.transact().map_err(move |err| {
                let new_err = err.map_db_err(|e| e.into());
                BlockValidationError::EVM { hash: tx.tx_hash(), error: Box::new(new_err) }
            })?;

            self.system_caller.on_state(&result_and_state.state);
            evm.db_mut().commit(result_and_state.state.clone());
            tx_results.push(SimulatedTx::new(tx.clone(), result_and_state, start_balance, block_coinbase));
        }

        let cache_state = std::mem::take(&mut evm.db_mut().cache);
        Ok((tx_results, cache_state))
    }

    /// Applies required state changes before transaction execution:
    /// - Sets state clear flag based on Spurious Dragon hardfork
    /// - Updates beacon root contract
    /// - Ensures create2deployer deployment at canyon transition
    fn apply_pre_execution_changes(
        &mut self,
        next_attributes: &NextBlockAttributes,
        evm: &mut Evm<'_, (), &mut State<DBFrag<Db>>>,
    ) -> Result<(), BlockExecutionError> {
        let block_number = u64::try_from(evm.block().number).unwrap();
        let block_timestamp = u64::try_from(evm.block().timestamp).unwrap();

        // Set state clear flag if the block is after the Spurious Dragon hardfork.
        evm.db_mut()
            .set_state_clear_flag(self.evm_config.chain_spec().is_spurious_dragon_active_at_block(block_number));

        self.system_caller.apply_beacon_root_contract_call(
            block_timestamp,
            block_number,
            next_attributes.parent_beacon_block_root,
            evm,
        )?;

        ensure_create2_deployer(self.evm_config.chain_spec().clone(), block_timestamp, evm.db_mut())
            .map_err(|_| OpBlockExecutionError::ForceCreate2DeployerFail)?;

        Ok(())
    }

    /// Constructs new block environment configuration from parent header and attributes
    fn get_env_for_new_block(&self, evm_block_params: &EvmBlockParams<Db>) -> EnvWithHandlerCfg {
        let EvmEnv { cfg_env_with_handler_cfg, block_env } = self
            .evm_config
            .next_cfg_and_block_env(&evm_block_params.parent_header, evm_block_params.attributes.env_attributes.clone())
            .expect("Valid block environment configuration");

        EnvWithHandlerCfg::new_with_cfg_env(cfg_env_with_handler_cfg, block_env, Default::default())
    }

    /// Updates internal EVM environments with new configuration
    #[inline]
    fn update_evm_environments(&mut self, env_with_handler_cfg: &EnvWithHandlerCfg) {
        self.evm_tof.modify_spec_id(env_with_handler_cfg.spec_id());
        self.evm_tof.context.evm.env = env_with_handler_cfg.env.clone();

        self.evm_sorting.modify_spec_id(env_with_handler_cfg.spec_id());
        self.evm_sorting.context.evm.env = env_with_handler_cfg.env.clone();
    }
}

impl<'a, Db: DatabaseRef + Clone> Actor<Db> for Simulator<'a, Db>
where
    <Db as DatabaseRef>::Error: Into<ProviderError> + Debug + Display,
{
    const CORE_AFFINITY: Option<usize> = None;

    fn loop_body(&mut self, connections: &mut SpineConnections<Db>) {
        // Received each new block from the sequencer.
        connections.receive(|msg, senders| {
            self.on_new_block(msg, senders);
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
