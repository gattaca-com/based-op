use std::{collections::HashMap, sync::Arc};

use alloy_consensus::Transaction as TransactionTrait;
use alloy_primitives::Address;
use bop_common::{
    communication::{Producer, SendersSpine, TrackedSenders, messages::SequencerToSimulator},
    db::{DBFrag, DatabaseRead},
    order::{
        Order, SimulatedOrder,
        bundle::{SimulatedBundle, ValidatedBundle},
    },
    telemetry::TelemetryUpdate,
    time::Duration,
    transaction::{SimulatedTxList, Transaction, TxList},
};
use reth_optimism_primitives::transaction::OpTransaction;
use reth_primitives_traits::InMemorySize;

use crate::transaction::pending::{PendingOrders, PendingOrdersView};

// TODO(mempirate): We need a way to understand if a bundle can be executed or not, and then
#[derive(Clone, Debug, Default)]
pub struct TxPool {
    /// maps an eoa to all pending txs
    pool_data: HashMap<Address, TxList>,
    /// Current list of all simulated mineable txs in the pool
    pub pending_orders: PendingOrders,
    /// Current memory size of the pool in bytes
    pub mem_size: usize,
}

impl TxPool {
    pub fn new(capacity: usize) -> Self {
        Self {
            pool_data: HashMap::with_capacity(capacity),
            pending_orders: PendingOrders::with_capacity(capacity),
            mem_size: 0,
        }
    }

    /// Handles a new [`Order`] from the sequencer.
    pub fn handle_order<Db: DatabaseRead>(
        &mut self,
        order: Order,
        db: &DBFrag<Db>,
        base_fee: u64,
        syncing: bool,
        sim_sender: Option<&SendersSpine<Db>>,
    ) {
        // We can't just call `handle_new_tx` here because we need to handle bundles differently. They must always
        // remain atomic, which means that we can not add individual txs to the pending list.
        match order {
            Order::Tx(tx) => {
                self.handle_tx(tx, db, base_fee, syncing, sim_sender);
            }
            Order::Bundle(bundle) => {
                self.handle_bundle(bundle, db, base_fee, syncing, sim_sender);
            }
        }
    }

    pub fn handle_bundle<Db: DatabaseRead>(
        &mut self,
        bundle: Arc<ValidatedBundle>,
        db: &DBFrag<Db>,
        base_fee: u64,
        syncing: bool,
        sim_sender: Option<&SendersSpine<Db>>,
    ) {
        if syncing {
            // Short-circuit here
            return;
        }

        // Simple transaction validation closure
        let validate_tx = |tx: &Transaction| {
            let state_nonce = db.get_nonce(tx.sender()).expect("failed to get nonce");
            let nonce = tx.nonce();

            // Only accept transactions with the correct nonce
            // TODO: We might want a bundle queueing mechanism.
            if nonce != state_nonce || !tx.valid_for_block(base_fee) {
                return false;
            }

            // Check the next nonce for this sender in the pending orders.
            // TODO(mempirate): This should compare effective gas prices, and swap if more profitable.
            let pending_nonce = self.pending_orders.next_nonce(tx.sender());
            if pending_nonce.is_some_and(|pending| pending == nonce) {
                return false;
            }

            true
        };

        // Validate all transactions in the bundle
        if !bundle.transactions.iter().all(validate_tx) {
            return;
        }

        self.mem_size = self.mem_size.saturating_add(bundle.size());
        self.pending_orders.put_bundle(SimulatedBundle::new(bundle));

        // TODO(mempirate): Request simulation for the bundle.
    }

    /// Handles an incoming transaction.
    /// Always adds to the pending list.
    ///
    /// If syncing is false we will fill the active list.
    /// If sim_sender is Some, and we are not syncing, we will also send simulation requests for the
    /// first tx for each sender to the simulator.
    pub fn handle_tx<Db: DatabaseRead>(
        &mut self,
        new_tx: Arc<Transaction>,
        db: &DBFrag<Db>,
        base_fee: u64,
        syncing: bool,
        sim_sender: Option<&SendersSpine<Db>>,
    ) -> bool {
        let state_nonce = db.get_nonce(new_tx.sender()).expect("handle failed db");
        let nonce = new_tx.nonce();
        // check nonce is valid
        if nonce < state_nonce {
            return false;
        }

        let is_next_nonce = nonce == state_nonce;

        // Add to pool and send to simulator if mineable
        match self.pool_data.get_mut(new_tx.sender_ref()) {
            Some(tx_list) => {
                // If it conflicts with a current tx compare effective gas prices, this also
                // overwrites if gas price is equal, taking into account conditions
                // above where we didn't return
                if tx_list.get_effective_price_for_nonce(&nonce, base_fee) > new_tx.effective_gas_price(Some(base_fee))
                {
                    return false;
                }
                tx_list.put(new_tx.clone());
                self.mem_size = self.mem_size.saturating_add(new_tx.size());

                // Check the next nonce for this sender in the pending orders.
                // TODO(mempirate): This should compare effective gas prices, and swap if more profitable.
                let pending_nonce = self.pending_orders.next_nonce(new_tx.sender());
                if pending_nonce.is_some_and(|pending| pending == nonce) {
                    return false;
                }

                if !syncing {
                    let valid_for_block = new_tx.valid_for_block(base_fee);
                    if is_next_nonce && valid_for_block {
                        // If this is the first tx for a sender, and it can be processed, simulate it and add to active.
                        TxPool::send_sim_requests_for_tx(&new_tx, db, sim_sender);
                        self.pending_orders.put_tx_list(SimulatedTxList::new(None, tx_list));
                        self.mem_size = self.mem_size.saturating_add(tx_list.mem_size());
                    } else if valid_for_block {
                        // If we already have the first tx for this sender and it's in active we might be able to
                        // add this tx to its pending list.
                        if let Some(simulated_tx_list) = self.pending_orders.tx_list_mut(new_tx.sender_ref()) {
                            if tx_list.nonce_ready(state_nonce, base_fee, nonce) {
                                simulated_tx_list.new_pending(tx_list.ready(state_nonce, base_fee).unwrap());
                            }
                        }
                    }
                }
            }
            None => {
                let tx_list = TxList::from(new_tx.clone());

                if !syncing {
                    // Check the next nonce for this sender in the pending orders.
                    // TODO(mempirate): This should compare effective gas prices, and swap if more profitable.
                    let pending_nonce = self.pending_orders.next_nonce(new_tx.sender());
                    if pending_nonce.is_some_and(|pending| pending == nonce) {
                        return false;
                    }

                    // If this is the first tx for a sender, and it can be processed, simulate it and add to active.
                    if is_next_nonce && new_tx.valid_for_block(base_fee) {
                        TxPool::send_sim_requests_for_tx(&new_tx, db, sim_sender);
                        self.pending_orders.put_tx_list(SimulatedTxList::new(None, &tx_list));
                        self.mem_size = self.mem_size.saturating_add(tx_list.mem_size());
                    }
                }

                self.mem_size = self.mem_size.saturating_add(new_tx.size());
                self.pool_data.insert(tx_list.sender(), tx_list);
            }
        }

        true
    }

    /// Validates simualted tx. If valid, fetch its TxList and save the new [SimulatedTxList] to `active_txs`.
    /// TODO(mempirate): Handle `SimulatedOrder` here
    pub fn handle_simulated(&mut self, order: SimulatedOrder) {
        match order {
            SimulatedOrder::Tx(simulated_tx) => {
                let Some(tx_list) = self.pool_data.get(simulated_tx.sender_ref()) else {
                    tracing::warn!(sender = ?simulated_tx.sender(), "Couldn't find tx list for valid simulated tx");
                    return;
                };

                // Refresh active txs with the latest tx_list and simulated tx.
                // TODO: probably unecassary to copy the tx_list here.
                let simulated_tx_list = SimulatedTxList::new(Some(simulated_tx), tx_list);
                let mem_size = simulated_tx_list.mem_size();
                self.pending_orders.put_tx_list(simulated_tx_list);
                self.mem_size = self.mem_size.saturating_add(mem_size);
            }

            SimulatedOrder::Bundle(simulated_bundle) => {
                tracing::debug!(id = %simulated_bundle.id(), "Received simulated bundle");
                self.pending_orders.put_bundle(simulated_bundle);
            }
        }
    }

    /// Removes a transaction with sender and nonce from the pool.
    pub fn remove(&mut self, sender: &Address, nonce: u64, telemetry_producer: &mut Producer<TelemetryUpdate>) {
        let mut f = |t: Arc<Transaction>| {
            self.mem_size = self.mem_size.saturating_sub(t.size());
            TelemetryUpdate::send(
                t.uuid,
                bop_common::telemetry::Telemetry::Tx(bop_common::telemetry::Tx::RemovedFromPool),
                telemetry_producer,
            )
        };

        if let Some(tx_list) = self.pool_data.get_mut(sender) {
            if tx_list.forward(nonce, &mut f) {
                self.pool_data.remove(sender);
            }
        }

        self.pending_orders.forward(sender, nonce, &mut f);
    }

    pub fn remove_mined_txs<'a, T: OpTransaction + TransactionTrait + 'a>(
        &mut self,
        mined_txs: impl Iterator<Item = (&'a Address, &'a T)>,
        telemetry_producer: &mut Producer<TelemetryUpdate>,
    ) {
        let mut f = |t: Arc<Transaction>| {
            self.mem_size = self.mem_size.saturating_sub(t.size());
            TelemetryUpdate::send(
                t.uuid,
                bop_common::telemetry::Telemetry::Tx(bop_common::telemetry::Tx::RemovedFromPool),
                telemetry_producer,
            )
        };

        // Clear all mined nonces from the pool
        for (sender, tx) in mined_txs {
            let nonce = tx.nonce();
            if let Some(sender_tx_list) = self.pool_data.get_mut(sender) {
                if sender_tx_list.forward(nonce, &mut f) {
                    self.pool_data.remove(sender);
                }
            }
            self.pending_orders.forward(sender, nonce, &mut f);
        }
    }

    /// Clears all mined txs from the pending list and resets the active list.
    /// This gets called in two places:
    /// 1) When we sync a new block.
    /// 2) When we commit a new Frag.
    pub fn handle_new_frag<Db: DatabaseRead>(
        &mut self,
        base_fee: u64,
        db: &DBFrag<Db>,
        syncing: bool,
        sim_sender: Option<&SendersSpine<Db>>,
    ) {
        // If enabled, fill the active list with non-simulated txs and send off the first tx for each sender to
        // simulator.
        if !syncing {
            self.pending_orders.clear();
            for (sender, tx_list) in self.pool_data.iter() {
                let db_nonce = db.get_nonce(*sender).unwrap();
                if let Some(ready) = tx_list.ready(db_nonce, base_fee) {
                    TxPool::send_sim_requests_for_tx(ready.peek().unwrap(), db, sim_sender);
                    self.pending_orders.put_tx_list(SimulatedTxList::new(None, tx_list));
                    self.mem_size = self.mem_size.saturating_add(tx_list.mem_size());
                }
            }
        }
    }

    fn send_sim_requests_for_tx<Db: DatabaseRead>(
        tx: &Arc<Transaction>,
        db: &DBFrag<Db>,
        sim_sender: Option<&SendersSpine<Db>>,
    ) {
        if let Some(sim_sender) = sim_sender {
            if let Err(error) = sim_sender
                .send_timeout(SequencerToSimulator::SimulateTxTof(tx.clone(), db.clone()), Duration::from_millis(10))
            {
                tracing::warn!(?error, "couldn't send simulator message");
                debug_assert!(false, "Couldn't send simulator message");
            }
        }
    }

    #[inline]
    pub fn snapshot(&self) -> PendingOrdersView {
        self.pending_orders.view()
    }

    #[inline]
    pub fn clear(&mut self) {
        self.pending_orders.clear();
        self.pool_data.clear();
        self.mem_size = 0;
    }
}
