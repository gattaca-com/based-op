use std::{fmt::Display, sync::Arc};

use bop_common::{
    communication::{
        messages::{SequencerToSimulator, SimulationResult},
        SendersSpine, SpineConnections, TrackedSenders,
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
use revm::{Database, DatabaseCommit, DatabaseRef};
use revm_primitives::{Address, EnvWithHandlerCfg, U256};
use tracing::error;

use super::FragSequence;
use crate::{context::SequencerContext, simulator::simulate_tx_inner, sorting::ActiveOrders};

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
        let db = DBSorting::new(data.db_frag.clone());
        let _  = ensure_create2_deployer(data.chain_spec().clone(), data.timestamp(), &mut db.db.write());
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
    pub fn handle_sim(&mut self, simulated_tx: SimulationResult<SimulatedTx>, sender: &Address, base_fee: u64) {
        self.in_flight_sims -= 1;

        tracing::trace!("handling sender {sender}");
        // handle errored sim
        let Ok(simulated_tx) = simulated_tx.inspect_err(|e| error!("simming tx for sender {sender} {e}")) else {
            self.tof_snapshot.remove_from_sender(sender, base_fee);
            return;
        };

        tracing::trace!("succesful for nonce {}", simulated_tx.nonce_ref());
        if self.gas_remaining < simulated_tx.gas_used() {
            self.tof_snapshot.remove_from_sender(sender, base_fee);
            return;
        }

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
        !self.is_empty() && (self.tof_snapshot.is_empty() || self.until < Instant::now())
    }

    pub fn should_send_next_sims(&self) -> bool {
        self.in_flight_sims == 0
    }
}

impl<Db: Clone + DatabaseRef> SortingData<Db> {
    pub fn apply_and_send_next(
        mut self,
        n_sims_per_loop: usize,
        senders: &mut SpineConnections<Db>,
        base_fee: u64,
    ) -> Self {
        self.maybe_apply(base_fee);

        let db = self.state();

        for t in self.tof_snapshot.iter().rev().take(n_sims_per_loop).map(|t| t.next_to_sim()) {
            debug_assert!(t.is_some(), "Unsimmable TxList should have been cleared previously");
            let tx = t.unwrap();
            tracing::trace!("sending sender {}, nonce {}", tx.sender(), tx.nonce_ref());
            // tracing::info!("sending sim {} for sender {}", tx.nonce_ref(), tx.sender());
            senders.send(SequencerToSimulator::SimulateTx(tx, db.clone()));
            self.in_flight_sims += 1;
        }
        self
    }

    pub fn state(&self) -> DBSorting<Db> {
        self.db.clone()
    }

    pub fn send_tx(&mut self, tx: Arc<Transaction>, senders: &SendersSpine<Db>) {
        let could_send = senders
            .send_timeout(SequencerToSimulator::SimulateTx(tx, self.db.clone()), Duration::from_millis(10))
            .is_ok();
        debug_assert!(could_send, "somehow simulate queue got filled");
        self.in_flight_sims += 1;
    }
}

impl<Db: DatabaseRef> SortingData<Db> {
    pub fn apply_tx(&mut self, mut tx: SimulatedTx) {
        self.db.commit(tx.take_state());

        let gas_used = tx.as_ref().result.gas_used();
        debug_assert!(self.gas_remaining > gas_used, "had too little gas remaining to apply tx {tx:#?}");

        tracing::trace!("applying sender {}, nonce {}", tx.sender(), tx.nonce_ref());

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
        let mut evm = evm_config.evm_with_env(&mut context.db_frag, env_with_handler_cfg);

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

        let forced_inclusion_txs = context.payload_attributes.transactions.as_ref().unwrap();

        // Apply must include txs.
        for tx in forced_inclusion_txs.iter() {
            let tx = Arc::new(Transaction::decode(tx.clone()).unwrap());

            // Execute transaction.
            let mut simulated_tx = simulate_tx_inner(tx, &mut evm, regolith_active, true, true)
                .expect("forced inclusing txs shouldn't fail");

            context.system_caller.on_state(&simulated_tx.result_and_state.state);
            evm.db_mut().commit(simulated_tx.take_state());
            self.gas_remaining -= simulated_tx.gas_used();
            self.payment += simulated_tx.payment;
            self.txs.push(simulated_tx);
        }
        Ok(())
    }
}
