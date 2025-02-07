use std::{
    fmt::{Debug, Display},
    sync::Arc,
};

use alloy_consensus::transaction::Transaction as TransactionTrait;
use bop_common::{
    actor::Actor,
    communication::{
        messages::{
            EvmBlockParams, NextBlockAttributes, SequencerToSimulator, SimulationError, SimulatorToSequencer,
            SimulatorToSequencerMsg,
        },
        SendersSpine, SpineConnections, TrackedSenders,
    },
    db::{DBFrag, DBSorting, DatabaseRead, State},
    time::Duration,
    transaction::{SimulatedTx, Transaction},
    utils::last_part_of_typename,
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
use reth_optimism_forks::OpHardfork;
use revm::{
    db::{states::bundle_state::BundleRetention, BundleState, CacheDB},
    Database, DatabaseCommit, DatabaseRef, Evm,
};
use revm_primitives::{Address, EnvWithHandlerCfg, EvmState};

/// Simulator thread.
///
/// TODO: need to impl fn to use system caller and return changes for that.
pub struct Simulator<'a, Db: DatabaseRef> {
    /// Top of frag evm.
    evm_tof: Evm<'a, (), State<DBFrag<Db>>>,

    /// Evm on top of partially built frag
    evm_sorting: Evm<'a, (), State<Arc<DBSorting<Db>>>>,

    /// Whether the regolith hardfork is active for the block that the evms are configured for.
    regolith_active: bool,

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
        let db_tof = State::new(db.clone());
        let evm_tof: Evm<'_, (), _> = evm_config.evm(db_tof);
        let db_sorting = State::new(Arc::new(DBSorting::new(db)));
        let evm_sorting: Evm<'_, (), _> = evm_config.evm(db_sorting);

        Self { evm_sorting, evm_tof, system_caller, evm_config: evm_config.clone(), id, regolith_active: true }
    }

    /// finalise
    fn simulate_tx<SimulateTxDb: DatabaseRef>(
        tx: Arc<Transaction>,
        db: SimulateTxDb,
        evm: &mut Evm<'a, (), State<SimulateTxDb>>,
        regolith_active: bool,
    ) -> Result<SimulatedTx, SimulationError> {
        // Cache some values pre-simulation.
        let coinbase = evm.block().coinbase;
        let start_balance = evm.db_mut().basic(coinbase).ok().flatten().map(|a| a.balance).unwrap_or_default();
        let depositor_nonce = (tx.is_deposit() && regolith_active)
            .then(|| evm.db_mut().basic(tx.sender()).ok().flatten().map(|a| a.nonce).unwrap_or_default());

        let old_db = std::mem::replace(evm.db_mut(), State::new(db));
        tx.fill_tx_env(evm.tx_mut());
        let res = evm.transact();
        // This dance is needed to drop the arc ref
        let _ = std::mem::replace(evm.db_mut(), old_db);
        let res = res.map_err(|_e| SimulationError::EvmError("todo 2".to_string()))?;

        Ok(SimulatedTx::new(tx, res, start_balance, coinbase, depositor_nonce))
    }

    /// Updates internal EVM environments with new configuration
    #[inline]
    fn update_evm_environments(&mut self, evm_block_params: EvmBlockParams) {
        let timestamp = u64::try_from(evm_block_params.env.block.timestamp).unwrap();
        self.evm_tof.modify_spec_id(evm_block_params.spec_id);
        self.evm_tof.context.evm.env = evm_block_params.env.clone();

        self.evm_sorting.modify_spec_id(evm_block_params.spec_id);
        self.evm_sorting.context.evm.env = evm_block_params.env;

        self.regolith_active = self.evm_config.chain_spec().fork(OpHardfork::Regolith).active_at_timestamp(timestamp);
    }
}

impl<Db: DatabaseRef + Clone> Actor<Db> for Simulator<'_, Db>
where
    Db: DatabaseRead + Database<Error: Into<ProviderError> + Display>,
{
    const CORE_AFFINITY: Option<usize> = None;

    fn name(&self) -> String {
        let name = last_part_of_typename::<Self>();
        format!("{}-{}", name, self.id)
    }

    fn loop_body(&mut self, connections: &mut SpineConnections<Db>) {
        // Received each new block from the sequencer.
        connections.receive(|msg, _| {
            self.update_evm_environments(msg);
        });

        connections.receive(|msg: SequencerToSimulator<Db>, senders| {
            match msg {
                // TODO: Cleanup: merge both functions?
                SequencerToSimulator::SimulateTx(tx, db) => {
                    let _ = senders.send_timeout(
                        SimulatorToSequencer::new(
                            (tx.sender(), tx.nonce()),
                            db.state_id(),
                            SimulatorToSequencerMsg::Tx(Self::simulate_tx(
                                tx,
                                db,
                                &mut self.evm_sorting,
                                self.regolith_active,
                            )),
                        ),
                        Duration::from_millis(10),
                    );
                }
                SequencerToSimulator::SimulateTxTof(tx, db) => {
                    let _ = senders.send_timeout(
                        SimulatorToSequencer::new(
                            (tx.sender(), tx.nonce()),
                            db.state_id(),
                            SimulatorToSequencerMsg::TxPoolTopOfFrag(Self::simulate_tx(
                                tx,
                                db,
                                &mut self.evm_tof,
                                self.regolith_active,
                            )),
                        ),
                        Duration::from_millis(10),
                    );
                }
            }
        });
    }
}
