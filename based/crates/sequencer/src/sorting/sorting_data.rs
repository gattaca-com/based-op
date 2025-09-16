use std::{
    fmt::{self, Display},
    ops::AddAssign,
    sync::Arc,
};

use bop_common::{
    communication::{
        Producer, SpineConnections,
        messages::{SequencerToSimulator, SimulationResult, SimulatorToSequencer, SimulatorToSequencerMsg},
    },
    db::{DBSorting, state::ensure_create2_deployer},
    metrics::{Counter, Gauge, Histogram, Metric, MetricsUpdate},
    telemetry::{Telemetry, TelemetryUpdate},
    time::{Duration, Instant},
    transaction::{SimulatedTx, Transaction},
    typedefs::{Database, DatabaseRef},
};
use bop_db::DatabaseRead;
use reth_chainspec::EthereumHardforks;
use reth_evm::{
    ConfigureEvm, Evm,
    block::StateChangeSource,
    execute::{BlockExecutionError, ProviderError},
};
use reth_optimism_evm::OpBlockExecutionError;
use reth_optimism_payload_builder::config::OpDAConfig;
use revm_primitives::{Address, U256};
use tracing::{debug, trace};
use uuid::Uuid;

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
    pub fn report(&self, metrics_producer: &Producer<MetricsUpdate>) {
        tracing::info!(
            "{} total sims: {}% success, tot simtime {}",
            self.n_sims_sent,
            if self.n_sims_sent == 0 {
                100.0
            } else {
                (self.n_sims_succesful * 10000 / self.n_sims_sent) as f64 / 100.0
            },
            self.tot_sim_time
        );

        MetricsUpdate::send_ref(
            Uuid::new_v4(),
            Metric::SetGauge(Gauge::GatewaySortingSimsSent, self.n_sims_sent as f64),
            metrics_producer,
        );
        MetricsUpdate::send_ref(
            Uuid::new_v4(),
            Metric::SetGauge(Gauge::GatewaySortingSimTime, self.tot_sim_time.as_millis()),
            metrics_producer,
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
    /// identifier for each frag that is sorted
    pub uuid: Uuid,
    /// Frag state with applied txs
    pub db: DBSorting<Db>,
    pub gas_remaining: u64,
    pub da_config: OpDAConfig,
    pub da_remaining: Option<u64>,
    pub payment: U256,
    pub txs: Vec<SimulatedTx>,
    /// Sort frag until, and then commit
    pub until: Instant,
    /// We wait until these are back before we apply the next
    /// and send the next round of simulations
    pub in_flight_sims: usize,
    /// Whether we should use fifo ordering instead of sorting by payments value.
    pub fifo_ordering: bool,
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
    pub telemetry_producer: Producer<TelemetryUpdate>,
    pub metrics_producer: Producer<MetricsUpdate>,
}

impl<Db: DatabaseRead> SortingData<Db> {
    pub fn new(seq: &FragSequence, data: &mut SequencerContext<Db>) -> Self
    where
        Db: Clone + DatabaseRef,
    {
        data.tx_pool.handle_new_frag(data.base_fee, data.shared_state.as_ref(), false, None);

        let tof_snapshot = if data.payload_attributes.no_tx_pool.unwrap_or_default() {
            ActiveOrders::empty()
        } else {
            ActiveOrders::new(data.tx_pool.clone_active())
        };
        let db = DBSorting::new(data.shared_state.as_ref().clone());
        let _ = ensure_create2_deployer(data.chain_spec().clone(), data.timestamp(), &mut db.db.write());
        let uuid = Uuid::new_v4();
        let mut telemetry_producer = data.telemetry;
        TelemetryUpdate::send(
            uuid,
            Telemetry::Frag(bop_common::telemetry::Frag::SorterStart {
                seq: seq.next_seq,
                block: data.block_number(),
                available_value: tof_snapshot.available_value().into(),
            }),
            &mut telemetry_producer,
        );

        debug!(da_remaining = seq.da_remaining, gas_remaining = seq.gas_remaining, "new sorting data created");

        Self {
            db,
            until: Instant::now() + data.config.frag_duration,
            in_flight_sims: 0,
            payment: U256::ZERO,
            next_to_be_applied: None,
            tof_snapshot,
            gas_remaining: seq.gas_remaining,
            da_config: data.config.da_config.clone(),
            da_remaining: seq.da_remaining,
            fifo_ordering: data.config.fifo_ordering,
            txs: vec![],
            start_t: Instant::now(),
            telemetry: Default::default(),
            uuid,
            telemetry_producer,
            metrics_producer: data.metrics,
        }
    }
}

impl<Db> SortingData<Db> {
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
        sender: Address,
        base_fee: u64,
        simtime: Duration,
    ) {
        self.in_flight_sims -= 1;
        self.telemetry.tot_sim_time += simtime;

        trace!("handling sender {sender}");
        // handle errored sim
        let Ok(simulated_tx) = simulated_tx.inspect_err(|e| {
            tracing::trace!("error {e} for tx: {}", sender);
            // Send metric for simulation error
            MetricsUpdate::send(
                self.uuid,
                Metric::IncrementCounter(Counter::SimulationErrors, 1),
                &mut self.metrics_producer,
            );
        }) else {
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

        // Send metrics for successful simulation result
        MetricsUpdate::send(
            self.uuid,
            Metric::IncrementCounter(Counter::SimulationResultsReceived, 1),
            &mut self.metrics_producer,
        );

        // Send metric for simulation latency
        MetricsUpdate::send(
            self.uuid,
            Metric::RecordHistogram(Histogram::SimulationLatencyMs, simtime.as_millis()),
            &mut self.metrics_producer,
        );

        let tx_to_put_back = if simulated_tx.gas_used() < self.gas_remaining &&
            self.next_to_be_applied.as_ref().is_none_or(|t| t.payment < simulated_tx.payment) &&
            !self.fifo_ordering
        {
            self.next_to_be_applied.replace(simulated_tx)
        } else {
            Some(simulated_tx)
        };
        if let Some(tx) = tx_to_put_back {
            self.tof_snapshot.put(tx, self.fifo_ordering)
        }
    }

    pub fn should_seal_frag(&self) -> bool {
        !self.is_empty() && self.until < Instant::now()
    }

    pub fn should_send_next_sims(&self) -> bool {
        self.in_flight_sims == 0
    }

    pub fn send_finished_telemetry(&mut self) {
        TelemetryUpdate::send(
            self.uuid,
            Telemetry::Frag(bop_common::telemetry::Frag::SorterFinish {
                success: true,
                payment: self.payment.into(),
                best_order_value: self.txs.iter().map(|t| t.payment).max().unwrap_or_default().into(),
                n_txs: self.txs.len(),
                gas_used: self.gas_used(),
            }),
            &mut self.telemetry_producer,
        );

        // Send metrics for fragment size and duration
        MetricsUpdate::send(
            self.uuid,
            Metric::SetGauge(Gauge::GatewayFragTxCount, self.txs.len() as f64),
            &mut self.metrics_producer,
        );

        let frag_duration = self.start_t.elapsed();
        MetricsUpdate::send(
            self.uuid,
            Metric::RecordHistogram(Histogram::FragSealEndToEndMs, frag_duration.as_millis()),
            &mut self.metrics_producer,
        );
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

    pub fn send_next(&mut self, n_sims_per_loop: usize, senders: &mut SpineConnections<Db>) {
        if self.tof_snapshot.is_empty() {
            return;
        }
        let mut i = self.tof_snapshot.len() - 1;
        let mut sims_sent = 0;
        while self.in_flight_sims < n_sims_per_loop {
            // check if tx DA is smaller than max allowed
            let too_big = self.tof_snapshot.da_too_big(i, self.da_remaining, self.da_config.max_da_tx_size());
            // check if we even have enough gas left for next order
            if too_big || self.tof_snapshot.not_enough_gas(i, self.gas_remaining) {
                self.tof_snapshot.swap_remove_back(i);
                if i == 0 {
                    break;
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
            sims_sent += 1;
            if i == 0 {
                break;
            }
            i -= 1;
        }

        // Send metrics for simulation requests
        if sims_sent > 0 {
            MetricsUpdate::send(
                self.uuid,
                Metric::IncrementCounter(Counter::SimulationRequestsSent, sims_sent),
                &mut self.metrics_producer,
            );
        }

        // Send metrics for simulation queue state
        MetricsUpdate::send(
            self.uuid,
            Metric::SetGauge(Gauge::SimulationQueueDepth, self.tof_snapshot.len() as f64),
            &mut self.metrics_producer,
        );
        MetricsUpdate::send(
            self.uuid,
            Metric::SetGauge(Gauge::SimulationInFlightCount, self.in_flight_sims as f64),
            &mut self.metrics_producer,
        );
    }

    pub fn state(&self) -> DBSorting<Db> {
        self.db.clone()
    }
}

impl<Db: DatabaseRef> SortingData<Db> {
    pub fn apply_tx(&mut self, tx: SimulatedTx) {
        self.db.commit_ref(&tx.result_and_state.state);
        TelemetryUpdate::send(
            tx.uuid,
            tx.to_included_telemetry(self.uuid, self.txs.len()),
            &mut self.telemetry_producer,
        );

        let sim_time = tx.sim_time;
        let gas_used = tx.as_ref().result.gas_used();
        debug_assert!(self.gas_remaining > gas_used, "had too little gas remaining to apply tx {tx:#?}");

        self.gas_remaining -= gas_used;
        self.da_remaining = self.da_remaining.map(|da| da.saturating_sub(tx.tx.estimated_tx_compressed_size()));
        self.txs.push(tx);

        // Send metrics for transaction processing
        MetricsUpdate::send(
            self.uuid,
            Metric::RecordHistogram(Histogram::TransactionProcessingEndToEndMs, sim_time.as_millis()),
            &mut self.metrics_producer,
        );
    }

    pub fn maybe_apply(&mut self, base_fee: u64) {
        if let Some(tx_to_apply) = std::mem::take(&mut self.next_to_be_applied) {
            self.tof_snapshot.remove_from_sender(tx_to_apply.sender(), base_fee);
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
        env_with_handler_cfg: reth_evm::EvmEnv<op_revm::OpSpecId>,
    ) -> Result<(), BlockExecutionError> {
        let timestamp = env_with_handler_cfg.block_env.timestamp;
        let block_number = env_with_handler_cfg.block_env.number;
        let parent_beacon_block_root = context.parent_beacon_block_root();
        let parent_hash = context.parent_hash();

        let state_clear_flag = context.config.evm_config.chain_spec().is_spurious_dragon_active_at_block(block_number);

        let regolith_active = context.regolith_active(timestamp);

        let evm_config = context.config.evm_config.clone();
        let chain_spec = context.config.evm_config.chain_spec().clone();

        // Configure new EVM to apply pre-execution and must include txs.
        let mut evm = evm_config.evm_with_env(context.shared_state.as_mut(), env_with_handler_cfg);

        // Apply pre-execution changes.
        evm.db_mut().db.write().set_state_clear_flag(state_clear_flag);

        context.system_caller.apply_blockhashes_contract_call(parent_hash, &mut evm)?;
        context.system_caller.apply_beacon_root_contract_call(parent_beacon_block_root, &mut evm)?;

        ensure_create2_deployer(chain_spec, timestamp, &mut evm.db_mut().db.write())
            .map_err(|_| OpBlockExecutionError::ForceCreate2DeployerFail)?;

        // Apply must include txs.
        let Some(forced_inclusion_txs) = context.payload_attributes.transactions.as_ref() else {
            return Ok(());
        };

        for (i, tx) in forced_inclusion_txs.iter().enumerate() {
            let tx = Arc::new(Transaction::decode(tx.clone()).unwrap());

            // Execute transaction.
            let mut simulated_tx = simulate_tx_inner(tx, &mut evm, regolith_active, true, true)
                .expect("forced inclusing txs shouldn't fail");

            context.system_caller.on_state(StateChangeSource::Transaction(i), &simulated_tx.result_and_state.state);

            evm.db_mut().commit_txs(std::iter::once(&mut simulated_tx));
            self.gas_remaining -= simulated_tx.gas_used();
            self.da_remaining =
                self.da_remaining.map(|da| da.saturating_sub(simulated_tx.tx.estimated_tx_compressed_size()));
            self.payment += simulated_tx.payment;
            self.txs.push(simulated_tx);
        }

        Ok(())
    }
}
