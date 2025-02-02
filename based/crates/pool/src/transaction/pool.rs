#![allow(unused)] // TODO: remove

use std::{collections::HashMap, sync::Arc};

use alloy_consensus::Transaction as TransactionTrait;
use alloy_primitives::Address;
use bop_common::{
    communication::{messages::SequencerToSimulator, SendersSpine, TrackedSenders},
    db::{BopDbRead, DBFrag},
    time::Duration,
    transaction::{SimulatedTx, SimulatedTxList, Transaction, TxList},
};
use revm::db::CacheDB;

use crate::transaction::active::Active;

#[derive(Clone, Debug, Default)]
pub struct TxPool {
    /// maps an eoa to all pending txs
    pool_data: HashMap<Address, TxList>,
    /// Current list of all simulated mineable txs in the pool
    active_txs: Active,
}

impl TxPool {
    pub fn new(capacity: usize) -> Self {
        Self { pool_data: HashMap::with_capacity(capacity), active_txs: Active::with_capacity(capacity) }
    }

    /// Handles an incoming transaction. If the sim_sender is None, the assumption is that we are not yet
    /// ready to send simulation for top of block simulation
    pub fn handle_new_tx<Db: BopDbRead>(
        &mut self,
        new_tx: Arc<Transaction>,
        db: &DBFrag<Db>,
        base_fee: u64,
        sim_sender: &SendersSpine<Db>,
    ) {
        let state_nonce = db.get_nonce(new_tx.sender());
        let nonce = new_tx.nonce();
        // check nonce is valid
        if nonce < state_nonce {
            return;
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
                    return;
                }

                tx_list.put(new_tx.clone());

                let valid_for_block = new_tx.valid_for_block(base_fee);
                if is_next_nonce && valid_for_block {
                    // If this is the first tx for a sender, and it can be processed, simulate it
                    TxPool::send_sim_requests_for_tx(&new_tx, db, sim_sender);
                } else if valid_for_block {
                    // If we already have the first tx for this sender and it's in active we might be able to
                    // add this tx to its pending list.
                    if let Some(simulated_tx_list) = self.active_txs.tx_list_mut(new_tx.sender_ref()) {
                        if tx_list.nonce_ready(state_nonce, base_fee, nonce) {
                            simulated_tx_list.new_pending(tx_list.ready(state_nonce, base_fee).unwrap());
                        }
                    }
                }
            }
            None => {
                // If this is the first tx for a sender, and it can be processed, simulate it
                if is_next_nonce && new_tx.valid_for_block(base_fee) {
                    TxPool::send_sim_requests_for_tx(&new_tx, db, sim_sender);
                }

                let tx_list = TxList::from(new_tx);
                self.pool_data.insert(tx_list.sender(), tx_list);
            }
        }
    }

    /// Validates simualted tx. If valid, fetch its TxList and save the new [SimulatedTxList] to `active_txs`.
    pub fn handle_simulated(&mut self, simulated_tx: SimulatedTx) {
        if !simulated_tx.result_and_state.result.is_success() {
            //TODO: @George is it correct that we should remove all the txs if tof fails?
            // or should we just wait till maybe a later stage
            self.remove(simulated_tx.sender_ref())
        }

        let Some(tx_list) = self.pool_data.get(simulated_tx.sender_ref()) else {
            tracing::warn!(sender = ?simulated_tx.sender(), "Couldn't find tx list for valid simulated tx");
            return;
        };

        let simulated_tx_list = SimulatedTxList::new(simulated_tx, tx_list);
        self.active_txs.put(simulated_tx_list);
    }

    pub fn remove(&mut self, sender: &Address) {
        let _ = self.pool_data.remove(sender);
    }

    fn handle_new_block<Db: BopDbRead>(
        &mut self,
        mined_txs: &[Arc<Transaction>],
        base_fee: u64,
        db: &DBFrag<Db>,
        sim_sender: &SendersSpine<Db>,
    ) {
        // Remove all mined txs from tx pool
        // We loop through backwards for a small efficiency boost here,
        // forward removes all nonces for sender lower than start so if a sender
        // has multiple txs in the block we only need to remove once.
        for tx in mined_txs.iter().rev() {
            if let Some(sender_tx_list) = self.pool_data.get_mut(tx.sender_ref()) {
                if sender_tx_list.forward(tx.nonce_ref()) {
                    self.pool_data.remove(tx.sender_ref());
                }
            }
        }

        // Clear the active list. This will get refreshed after the sim results sent below come back.
        self.active_txs.clear();

        // Send next nonce for each active sender to simulator
        for (sender, sender_txs) in self.pool_data.iter() {
            let db_nonce = db.get_nonce(*sender);
            if let Some(first_tx) = sender_txs.first_ready(db_nonce, base_fee) {
                TxPool::send_sim_requests_for_tx(first_tx, db, sim_sender);
            }
        }
    }

    /// If this is called with `None` the assumption is that we are not yet ready to send top-of-block sims.
    fn send_sim_requests_for_tx<Db: BopDbRead>(tx: &Arc<Transaction>, db: &DBFrag<Db>, sim_sender: &SendersSpine<Db>) {
        if let Err(error) = sim_sender
            .send_timeout(SequencerToSimulator::SimulateTxTof(tx.clone(), db.clone()), Duration::from_millis(10))
        {
            tracing::warn!(?error, "couldn't send simulator message");
            debug_assert!(false, "Couldn't send simulator message");
        }
    }

    #[inline]
    pub fn clone_active(&self) -> Vec<SimulatedTxList> {
        self.active_txs.clone_txs()
    }

    #[inline]
    pub fn active(&self) -> &[SimulatedTxList] {
        self.active_txs.txs()
    }

    #[inline]
    pub fn num_active_txs(&self) -> usize {
        self.active_txs.num_txs()
    }

    #[inline]
    pub fn active_empty(&self) -> bool {
        self.active_txs.is_empty()
    }
}
