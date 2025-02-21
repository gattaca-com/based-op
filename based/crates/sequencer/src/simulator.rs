use std::{
    fmt::{Debug, Display},
    sync::Arc,
};

use bop_common::{
    actor::Actor,
    communication::{
        messages::{
            EvmBlockParams, SequencerToSimulator, SimulationError, SimulatorToSequencer, SimulatorToSequencerMsg,
        },
        SpineConnections, TrackedSenders,
    },
    db::{DBFrag, DBSorting, DatabaseRead, State},
    time::{Duration, Instant},
    transaction::{SimulatedTx, Transaction},
    utils::last_part_of_typename,
};
use reth_evm::{execute::ProviderError, ConfigureEvm};
use reth_optimism_evm::OpEvmConfig;
use reth_optimism_forks::OpHardfork;
use revm::{Database, DatabaseRef, Evm};
use revm_primitives::{Address, U256};

/// Simulator thread.
pub struct Simulator<'a, Db: DatabaseRef> {
    /// Top of frag evm.
    evm_tof: Evm<'a, (), State<DBFrag<Db>>>,
    /// Evm on top of partially built frag
    pub evm_sorting: Evm<'a, (), State<DBSorting<Db>>>,
    /// Whether the regolith hardfork is active for the block that the evms are configured for.
    regolith_active: bool,
    /// How to create an EVM.
    evm_config: OpEvmConfig,
    id: usize,
}

impl<'a, Db: DatabaseRef + Clone> Simulator<'a, Db>
where
    <Db as DatabaseRef>::Error: Into<ProviderError> + Debug + Display,
{
    pub fn new(db: DBFrag<Db>, evm_config: &'a OpEvmConfig, id: usize) -> Self {
        // Initialise with default evms. These will be overridden before the first sim by
        // `set_evm_for_new_block`.
        let db_tof = State::new(db.clone());
        let evm_tof: Evm<'_, (), _> = evm_config.evm(db_tof);
        let db_sorting = State::new(DBSorting::new(db));
        let evm_sorting: Evm<'_, (), _> = evm_config.evm(db_sorting);

        Self { evm_sorting, evm_tof, evm_config: evm_config.clone(), id, regolith_active: true }
    }

    /// Simulates a transaction at the state of the `db` parameter.
    pub fn simulate_transaction<SimulateTxDb: DatabaseRef>(
        tx: Arc<Transaction>,
        db: SimulateTxDb,
        evm: &mut Evm<'a, (), State<SimulateTxDb>>,
        regolith_active: bool,
        allow_zero_payment: bool,
        allow_revert: bool,
    ) -> Result<SimulatedTx, SimulationError>
    where
        SimulateTxDb::Error: std::fmt::Debug,
    {
        let _ = std::mem::replace(evm.db_mut(), State::new(db));
        simulate_tx_inner(tx, evm, regolith_active, allow_zero_payment, allow_revert)
    }

    /// Updates internal EVM environments with new configuration
    #[inline]
    pub fn update_evm_environments(&mut self, evm_block_params: EvmBlockParams) {
        let timestamp = u64::try_from(evm_block_params.env.block.timestamp).unwrap();
        self.evm_tof.modify_spec_id(evm_block_params.spec_id);
        self.evm_tof.context.evm.env = evm_block_params.env.clone();

        self.evm_sorting.modify_spec_id(evm_block_params.spec_id);
        self.evm_sorting.context.evm.env = evm_block_params.env;

        self.regolith_active = self.evm_config.chain_spec().fork(OpHardfork::Regolith).active_at_timestamp(timestamp);
    }
}

/// Simulates a transaction at the passed in EVM's state.
/// Will not modify the db state after the simulation is complete.
pub fn simulate_tx_inner<Db>(
    tx: Arc<Transaction>,
    evm: &mut Evm<'_, (), Db>,
    regolith_active: bool,
    allow_zero_payment: bool,
    allow_revert: bool,
) -> Result<SimulatedTx, SimulationError>
where
    Db: Database,
    Db::Error: std::fmt::Debug,
{
    let coinbase = evm.block().coinbase;
    // Cache some values pre-simulation.
    let start_balance = balance_from_db(evm.db_mut(), coinbase);
    let deposit_nonce = (tx.is_deposit() && regolith_active).then(|| nonce_from_db(evm.db_mut(), tx.sender()));

    // Prepare and execute the tx.
    tx.fill_tx_env(evm.tx_mut());
    let result_and_state = evm.transact().map_err(|e| SimulationError::EvmError(format!("{e:?}")))?;

    if !allow_revert && !result_and_state.result.is_success() {
        return Err(SimulationError::RevertWithDisallowedRevert);
    }

    // Determine payment tx made to the coinbase.
    let end_balance = result_and_state.state.get(&coinbase).map(|a| a.info.balance).unwrap_or_default();
    let payment = end_balance.saturating_sub(start_balance);

    if !allow_zero_payment && payment == U256::ZERO {
        return Err(SimulationError::ZeroPayment);
    }

    Ok(SimulatedTx::new(tx, result_and_state, payment, deposit_nonce))
}

#[inline]
fn nonce_from_db(db: &mut impl Database, address: Address) -> u64 {
    db.basic(address).ok().flatten().map(|a| a.nonce).unwrap_or_default()
}

#[inline]
fn balance_from_db(db: &mut impl Database, address: Address) -> U256 {
    db.basic(address).ok().flatten().map(|a| a.balance).unwrap_or_default()
}

impl<Db: DatabaseRef + Clone> Actor<Db> for Simulator<'_, Db>
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
                        Self::simulate_transaction(tx, db, &mut self.evm_sorting, self.regolith_active, true, true),
                    ),
                    SequencerToSimulator::SimulateTxTof(tx, db) => SimulatorToSequencerMsg::TxPoolTopOfFrag(
                        Self::simulate_transaction(tx, db, &mut self.evm_tof, self.regolith_active, true, true),
                    ),
                };
            let _ = senders.send_timeout(
                SimulatorToSequencer::new((sender, nonce), state_id, curt.elapsed(), msg),
                Duration::from_millis(10),
            );
        });
    }
}
