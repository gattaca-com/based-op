use std::{
    fmt::{Debug, Display},
    sync::Arc,
};

use bop_common::{
    actor::Actor,
    communication::{
        SpineConnections, TrackedSenders,
        messages::{SequencerToSimulator, SimulationError, SimulatorToSequencer, SimulatorToSequencerMsg},
    },
    db::{DBFrag, DBSorting, DatabaseRead, State},
    time::{Duration, Instant, Nanos},
    transaction::{SimulatedTx, Transaction},
    typedefs::*,
    utils::last_part_of_typename,
};
use op_revm::OpSpecId;
use reth_evm::{ConfigureEvm, Evm, EvmEnv, execute::ProviderError};
use reth_optimism_evm::{OpEvm, OpEvmConfig};
use reth_optimism_forks::OpHardfork;
use revm::context::{Block, DBErrorMarker};
use revm_inspector::NoOpInspector;
use revm_primitives::{Address, U256};

pub trait SimulationDatabase: DatabaseRef<
        Error: Send + Sync + 'static + DBErrorMarker + std::error::Error + Into<ProviderError> + Debug + Display,
    > + Database<Error: Send + Sync + 'static + DBErrorMarker + std::error::Error + Into<ProviderError> + Debug + Display>
{
}

impl<T> SimulationDatabase for T where
    T: DatabaseRef<
            Error: Send + Sync + 'static + DBErrorMarker + std::error::Error + Into<ProviderError> + Debug + Display,
        > + Database<
            Error: Send + Sync + 'static + DBErrorMarker + std::error::Error + Into<ProviderError> + Debug + Display,
        >
{
}

/// Simulator thread.
pub struct Simulator<Db: DatabaseRef<Error: Send + Sync + 'static + DBErrorMarker + std::error::Error>> {
    /// Top of frag evm.
    evm_tof: OpEvm<State<DBFrag<Db>>, NoOpInspector>,
    /// Evm on top of partially built frag
    pub evm_sorting: OpEvm<State<DBSorting<Db>>, NoOpInspector>,
    /// Whether the regolith hardfork is active for the block that the evms are configured for.
    regolith_active: bool,
    /// How to create an EVM.
    evm_config: OpEvmConfig,
    allow_reverts: bool,
    id: usize,
}

impl<
    Db: Database
        + DatabaseRef<
            Error: Send + Sync + 'static + DBErrorMarker + std::error::Error + Into<ProviderError> + Debug + Display,
        > + Clone,
> Simulator<Db>
{
    pub fn new(db: DBFrag<Db>, evm_config: OpEvmConfig, id: usize, allow_reverts: bool) -> Self {
        // Initialise with default evms. These will be overridden before the first sim by
        // `set_evm_for_new_block`.
        let db_tof = State::new(db.clone());
        let evm_tof = evm_config.evm_with_env(db_tof, EvmEnv::default());
        let db_sorting = State::new(DBSorting::new(db));
        let evm_sorting = evm_config.evm_with_env(db_sorting, EvmEnv::default());

        Self { evm_sorting, evm_tof, evm_config: evm_config.clone(), id, regolith_active: true, allow_reverts } 
    }

    /// Simulates a transaction at the state of the `db` parameter.
    pub fn simulate_transaction<SimulateTxDb: SimulationDatabase>(
        tx: Arc<Transaction>,
        db: SimulateTxDb,
        evm: &mut OpEvm<State<SimulateTxDb>, NoOpInspector>,
        regolith_active: bool,
        allow_zero_payment: bool,
        allow_revert: bool,
    ) -> Result<SimulatedTx, SimulationError> {
        let _ = std::mem::replace(evm.db_mut(), State::new(db));
        simulate_tx_inner(tx, evm, regolith_active, allow_zero_payment, allow_revert)
    }

    /// Updates internal EVM environments with new configuration
    #[inline]
    pub fn update_evm_environments(&mut self, evm_block_params: EvmEnv<OpSpecId>) {
        let timestamp = evm_block_params.block_env.timestamp();
        self.evm_tof.modify_block(|b| *b = evm_block_params.block_env.clone()); // TODO: re-use mem
        self.evm_tof.modify_cfg(|c| *c = evm_block_params.cfg_env.clone());
        self.evm_sorting.modify_block(|b| *b = evm_block_params.block_env.clone()); // TODO: re-use mem
        self.evm_sorting.modify_cfg(|c| *c = evm_block_params.cfg_env.clone());

        self.regolith_active = self.evm_config.chain_spec().fork(OpHardfork::Regolith).active_at_timestamp(timestamp);
    }
}

/// Simulates a transaction at the passed in EVM's state.
/// Will not modify the db state after the simulation is complete.
pub fn simulate_tx_inner<Db: SimulationDatabase>(
    tx: Arc<Transaction>,
    evm: &mut OpEvm<Db, NoOpInspector>,
    regolith_active: bool,
    allow_zero_payment: bool,
    allow_revert: bool,
) -> Result<SimulatedTx, SimulationError> {
    let coinbase = evm.block().beneficiary;
    let sim_start = Nanos::now();
    // Cache some values pre-simulation.
    let start_balance = balance_from_db(evm.db_mut(), coinbase);
    let deposit_nonce = (tx.is_deposit() && regolith_active).then(|| nonce_from_db(evm.db_mut(), tx.sender()));

    // Prepare and execute the tx.
    let result_and_state =
        evm.transact_raw(tx.to_op_tx_env()).map_err(|e| SimulationError::EvmError(format!("{e:?}")))?;

    if !allow_revert && !result_and_state.result.is_success() {
        return Err(SimulationError::RevertWithDisallowedRevert);
    }

    // Determine payment tx made to the coinbase.
    let end_balance = result_and_state.state.get(&coinbase).map(|a| a.info.balance).unwrap_or_default();
    let payment = end_balance.saturating_sub(start_balance);

    if !allow_zero_payment && payment == U256::ZERO {
        return Err(SimulationError::ZeroPayment);
    }

    Ok(SimulatedTx::new(tx, result_and_state, payment, deposit_nonce, sim_start.elapsed()))
}

#[inline]
fn nonce_from_db(db: &mut impl Database, address: Address) -> u64 {
    db.basic(address).ok().flatten().map(|a| a.nonce).unwrap_or_default()
}

#[inline]
fn balance_from_db(db: &mut impl Database, address: Address) -> U256 {
    db.basic(address).ok().flatten().map(|a| a.balance).unwrap_or_default()
}

impl<
    Db: DatabaseRef<
            Error: Send + Sync + 'static + DBErrorMarker + std::error::Error + Into<ProviderError> + Debug + Display,
        > + Clone,
> Actor<Db> for Simulator<Db>
where
    Db: DatabaseRead + Database<Error: Into<ProviderError> + Display>,
{
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
            let (sender, nonce, state_id) = msg.sim_info();
            let curt = Instant::now();
            let msg =
                match msg {
                    SequencerToSimulator::SimulateTx(tx, db) => SimulatorToSequencerMsg::Tx(
                        Self::simulate_transaction(tx, db, &mut self.evm_sorting, self.regolith_active, true, self.allow_reverts),
                    ),
                    SequencerToSimulator::SimulateTxTof(tx, db) => SimulatorToSequencerMsg::TxPoolTopOfFrag(
                        Self::simulate_transaction(tx, db, &mut self.evm_tof, self.regolith_active, true, self.allow_reverts),
                    ),
                };
            let _ = senders.send_timeout(
                SimulatorToSequencer::new((sender, nonce), state_id, curt.elapsed(), msg),
                Duration::from_millis(10),
            );
        });
    }
}
