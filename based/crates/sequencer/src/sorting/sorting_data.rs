use std::{
    fmt::{self, Display},
    ops::AddAssign,
    sync::Arc,
};

use bop_common::{
    communication::{
        messages::{SequencerToSimulator, SimulationResult, SimulatorToSequencer, SimulatorToSequencerMsg},
        SpineConnections,
    },
    db::{state::ensure_create2_deployer, DBSorting},
    time::{Duration, Instant},
    transaction::{SimulatedTx, Transaction},
};
use bop_db::DatabaseRead;
use reth_chainspec::EthereumHardforks;
use reth_evm::{
    execute::{BlockExecutionError, ProviderError},
    ConfigureEvm,
};
use reth_optimism_evm::OpBlockExecutionError;
use revm::{Database, DatabaseRef};
use revm_primitives::{Address, EnvWithHandlerCfg, U256};
use tracing::trace;

use super::FragSequence;
use crate::{context::SequencerContext, simulator::simulate_tx_inner, sorting::ActiveOrders};

#[derive(Clone, Copy, Default)]
pub struct SortingTelemetry {
    n_sims_sent: usize,
    n_sims_errored: usize,
    n_sims_succesful: usize,
    tot_sim_time: Duration,
}
impl SortingTelemetry {
    #[tracing::instrument(skip_all, name = "sorting_telemetry")]
    pub fn report(&self) {
        tracing::info!(
            "{} total sims: {}% success, tot simtime {}",
            self.n_sims_sent,
            (self.n_sims_succesful * 10000 / self.n_sims_errored.max(1)) as f64 / 100.0,
            self.tot_sim_time
        );
    }
}

impl AddAssign for SortingTelemetry {
    fn add_assign(&mut self, rhs: Self) {
        self.n_sims_sent += rhs.n_sims_sent;
        self.n_sims_errored += rhs.n_sims_errored;
        self.n_sims_succesful += rhs.n_sims_succesful;
        self.tot_sim_time += rhs.tot_sim_time;
    }
}

impl fmt::Debug for SortingTelemetry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SortingTelemetry")
            .field("n_sims_sent", &self.n_sims_sent)
            .field("n_sims_errored", &self.n_sims_errored)
            .field("n_sims_succesful", &self.n_sims_succesful)
            .field("tot_sim_time", &format_args!("{}", self.tot_sim_time))
            .finish()
    }
}

/// Data of a being sorted frag
#[derive(Clone, Debug)]
pub struct SortingData<Db> {
    /// Current frag being sorted
    pub db: DBSorting<Db>,
    pub gas_remaining: u64,
    pub payment: U256,
    pub txs: Vec<SimulatedTx>,
    /// Sort frag until, and then commit
    pub until: Instant,
    /// We wait until these are back before we apply the next
    /// and send the next round of simulations
    pub in_flight_sims: usize,
    /// Remaining orders to be sorted, ideally with top of frag (TOF)
    /// sim data. The TOF sim data can be used as a heuristic initial sort of
    /// the orders. The assumption is that applying some orders will not
    /// dramatically increase the value of an order vs its TOF value.
    /// This allows us to no have to fully resim all remaining orders
    /// every time we apply one, leading to a huge efficiency gain.
    pub tof_snapshot: ActiveOrders,
    /// While sim results come back, we keep track of the most valuable one here.
    /// If when all results are back (i.e. `in_flight_sims == 0`) this is Some,
    /// we apply it to the `db` and send off the next batch of sims.
    pub next_to_be_applied: Option<SimulatedTx>,

    pub start_t: Instant,

    pub telemetry: SortingTelemetry,
}

impl<Db> SortingData<Db> {
    pub fn new(seq: &FragSequence, data: &SequencerContext<Db>) -> Self
    where
        Db: Clone + DatabaseRef,
    {
        let tof_snapshot = if data.payload_attributes.no_tx_pool.unwrap_or_default() {
            ActiveOrders::empty()
        } else {
            ActiveOrders::new(data.tx_pool.clone_active())
        };
        let db = DBSorting::new(data.shared_state.as_ref().clone());
        let _ = ensure_create2_deployer(data.chain_spec().clone(), data.timestamp(), &mut db.db.write());

        Self {
            db,
            until: Instant::now() + data.config.frag_duration,
            in_flight_sims: 0,
            payment: U256::ZERO,
            next_to_be_applied: None,
            tof_snapshot,
            gas_remaining: seq.gas_remaining,
            txs: vec![],
            start_t: Instant::now(),
            telemetry: Default::default(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.txs.is_empty()
    }

    pub fn gas_used(&self) -> u64 {
        self.txs.iter().map(|t| t.gas_used()).sum()
    }

    pub fn payment(&self) -> U256 {
        self.payment
    }

    pub fn is_valid(&self, state_id: u64) -> bool {
        state_id == self.db.state_id()
    }

    /// Handles the result of a simulation. `simulated_tx` simulated_at_id should be pre-verified.
    pub fn handle_sim(
        &mut self,
        simulated_tx: SimulationResult<SimulatedTx>,
        sender: &Address,
        base_fee: u64,
        simtime: Duration,
    ) {
        self.in_flight_sims -= 1;
        self.telemetry.tot_sim_time += simtime;

        trace!("handling sender {sender}");
        // handle errored sim
        let Ok(simulated_tx) = simulated_tx.inspect_err(|e| tracing::trace!("error {e} for tx: {}", sender)) else {
            self.tof_snapshot.remove_from_sender(sender, base_fee);
            self.telemetry.n_sims_errored += 1;
            return;
        };

        trace!("succesful for nonce {}", simulated_tx.nonce_ref());
        if self.gas_remaining < simulated_tx.gas_used() {
            self.tof_snapshot.remove_from_sender(sender, base_fee);
            return;
        }
        self.telemetry.n_sims_succesful += 1;

        let tx_to_put_back = if simulated_tx.gas_used() < self.gas_remaining &&
            self.next_to_be_applied.as_ref().is_none_or(|t| t.payment < simulated_tx.payment)
        {
            self.next_to_be_applied.replace(simulated_tx)
        } else {
            Some(simulated_tx)
        };
        if let Some(tx) = tx_to_put_back {
            self.tof_snapshot.put(tx)
        }
    }

    pub fn should_seal_frag(&self) -> bool {
        !self.is_empty() && self.until < Instant::now()
    }

    pub fn should_send_next_sims(&self) -> bool {
        self.in_flight_sims == 0
    }
}

impl<Db: Clone + DatabaseRef> SortingData<Db> {
    pub fn handle_deposits(
        &mut self,
        deposits: &mut std::collections::VecDeque<Arc<Transaction>>,
        connections: &mut SpineConnections<Db>,
    ) {
        //TODO: we should do this inline
        while let Some(deposit) = deposits.pop_front() {
            connections.send(SequencerToSimulator::SimulateTx(deposit, self.state()));
            self.telemetry.n_sims_sent += 1;
            let mut found = false;
            while !found {
                connections.receive(|msg: SimulatorToSequencer, _| {
                    let state_id = msg.state_id;
                    self.telemetry.tot_sim_time += msg.simtime;
                    if !self.is_valid(state_id) {
                        return;
                    }

                    let SimulatorToSequencerMsg::Tx(simulated_tx) = msg.msg else {
                        debug_assert!(false, "this should always be tx, is something else running?");
                        return;
                    };
                    let Ok(res) = simulated_tx else {
                        self.telemetry.n_sims_errored += 1;
                        return;
                    };
                    self.telemetry.n_sims_succesful += 1;
                    found = true;

                    debug_assert!(res.tx.is_deposit(), "somehow found a valid sim that wasn't a deposit");
                    self.apply_tx(res);
                });
            }
        }
    }

    pub fn send_next(mut self, n_sims_per_loop: usize, senders: &mut SpineConnections<Db>) -> Self {
        if self.tof_snapshot.len() == 0 {
            return self;
        }
        let mut i = self.tof_snapshot.len() - 1;
        while self.in_flight_sims < n_sims_per_loop {
            // check if we even have enough gas left for next order
            if self.tof_snapshot.verify_gas(i, self.gas_remaining) {
                self.tof_snapshot.swap_remove_back(i);
                if i == 0 {
                    return self;
                }
                i -= 1;
                continue;
            }
            let order = self.tof_snapshot[i].next_to_sim();
            debug_assert!(order.is_some(), "Unsimmable TxList should have been cleared previously");
            let tx_to_sim = order.unwrap();
            senders.send(SequencerToSimulator::SimulateTx(tx_to_sim, self.state()));
            self.in_flight_sims += 1;
            self.telemetry.n_sims_sent += 1;
            if i == 0 {
                return self;
            }
            i -= 1;
        }

        self
    }

    pub fn state(&self) -> DBSorting<Db> {
        self.db.clone()
    }
}

impl<Db: DatabaseRef> SortingData<Db> {
    pub fn apply_tx(&mut self, tx: SimulatedTx) {
        self.db.commit_ref(&tx.result_and_state.state);

        let gas_used = tx.as_ref().result.gas_used();
        debug_assert!(self.gas_remaining > gas_used, "had too little gas remaining to apply tx {tx:#?}");

        self.gas_remaining -= gas_used;
        self.txs.push(tx);
    }

    pub fn maybe_apply(&mut self, base_fee: u64) {
        if let Some(tx_to_apply) = std::mem::take(&mut self.next_to_be_applied) {
            self.tof_snapshot.remove_from_sender(&tx_to_apply.sender(), base_fee);
            self.apply_tx(tx_to_apply);
        }
    }
}

impl<Db: DatabaseRead + Database<Error: Into<ProviderError> + Display>> SortingData<Db> {
    /// Must be called each new block.
    /// Applies pre-execution changes and must include txs from the payload attributes to the
    /// dbfrag that will be to sort/create all frags of this block on top of.
    ///
    /// Returns FragSequence and SortingData for this block.
    /// The former keeps track of all txs of this block (i.e. in a sequence of frags).
    /// After this function it contains the forced inclusion txs.
    pub fn apply_block_start_to_state(
        &mut self,
        context: &mut SequencerContext<Db>,
        env_with_handler_cfg: EnvWithHandlerCfg,
    ) -> Result<(), BlockExecutionError> {
        let timestamp = env_with_handler_cfg.block.timestamp.to();
        let block_number = env_with_handler_cfg.block.number.to();

        let should_set_state_clear_flag =
            context.config.evm_config.chain_spec().is_spurious_dragon_active_at_block(block_number);

        let parent_beacon_block_root = context.parent_beacon_block_root();

        let regolith_active = context.regolith_active(timestamp);

        let evm_config = context.config.evm_config.clone();
        let chain_spec = context.config.evm_config.chain_spec().clone();
        // Configure new EVM to apply pre-execution and must include txs.
        let mut evm = evm_config.evm_with_env(context.shared_state.as_mut(), env_with_handler_cfg);

        // Apply pre-execution changes.
        evm.db_mut().db.write().set_state_clear_flag(should_set_state_clear_flag);

        context.system_caller.apply_beacon_root_contract_call(
            timestamp,
            block_number,
            parent_beacon_block_root,
            &mut evm,
        )?;
        ensure_create2_deployer(chain_spec, timestamp, &mut evm.db_mut().db.write())
            .map_err(|_| OpBlockExecutionError::ForceCreate2DeployerFail)?;

        let Some(forced_inclusion_txs) = context.payload_attributes.transactions.as_ref() else {
            return Ok(());
        };

        // Apply must include txs.
        for tx in forced_inclusion_txs.iter() {
            let tx = Arc::new(Transaction::decode(tx.clone()).unwrap());

            // Execute transaction.
            let mut simulated_tx = simulate_tx_inner(tx, &mut evm, regolith_active, true, true)
                .expect("forced inclusing txs shouldn't fail");

            context.system_caller.on_state(&simulated_tx.result_and_state.state);

            evm.db_mut().commit_txs(std::iter::once(&mut simulated_tx));
            self.gas_remaining -= simulated_tx.gas_used();
            self.payment += simulated_tx.payment;
            self.txs.push(simulated_tx);
        }
        Ok(())
    }
}
