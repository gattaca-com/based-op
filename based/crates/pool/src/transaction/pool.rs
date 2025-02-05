#![allow(unused)] // TODO: remove

use std::{collections::HashMap, sync::Arc};

use alloy_consensus::Transaction as TransactionTrait;
use alloy_primitives::Address;
use bop_common::{
    communication::{messages::SequencerToSimulator, Sender, SendersSpine, TrackedSenders},
    db::{DBFrag, DatabaseRead},
    time::Duration,
    transaction::{SimulatedTx, SimulatedTxList, Transaction, TxList},
};
use reth_optimism_primitives::{transaction::TransactionSenderInfo, OpTransactionSigned};
use reth_primitives_traits::transaction::signed::SignedTransaction;
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

    /// Handles an incoming transaction.
    /// Always adds to the pending list.
    ///
    /// If syncing is false we will fill the active list.
    /// If sim_sender is Some, and we are not syncing, we will also send simulation requests for the
    /// first tx for each sender to the simulator.
    pub fn handle_new_tx<Db: DatabaseRead>(
        &mut self,
        new_tx: Arc<Transaction>,
        db: &DBFrag<Db>,
        base_fee: u64,
        syncing: bool,
        sim_sender: Option<&SendersSpine<Db>>,
    ) {
        let state_nonce = db.get_nonce(new_tx.sender()).expect("handle failed db");
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

                if !syncing {
                    let valid_for_block = new_tx.valid_for_block(base_fee);
                    if is_next_nonce && valid_for_block {
                        // If this is the first tx for a sender, and it can be processed, simulate it and add to active.
                        TxPool::send_sim_requests_for_tx(&new_tx, db, sim_sender);
                        self.active_txs.put(SimulatedTxList::new(None, tx_list));
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
            }
            None => {
                let tx_list = TxList::from(new_tx.clone());

                if !syncing {
                    // If this is the first tx for a sender, and it can be processed, simulate it and add to active.
                    if is_next_nonce && new_tx.valid_for_block(base_fee) {
                        TxPool::send_sim_requests_for_tx(&new_tx, db, sim_sender);
                        self.active_txs.put(SimulatedTxList::new(None, &tx_list));
                    }
                }

                self.pool_data.insert(tx_list.sender(), tx_list);
            }
        }
    }

    /// Validates simualted tx. If valid, fetch its TxList and save the new [SimulatedTxList] to `active_txs`.
    pub fn handle_simulated(&mut self, simulated_tx: SimulatedTx) {
        let Some(tx_list) = self.pool_data.get(simulated_tx.sender_ref()) else {
            tracing::warn!(sender = ?simulated_tx.sender(), "Couldn't find tx list for valid simulated tx");
            return;
        };

        // Refresh active txs with the latest tx_list and simulated tx.
        // TODO: probably unecassary to copy the tx_list here.
        let simulated_tx_list = SimulatedTxList::new(Some(simulated_tx), tx_list);
        self.active_txs.put(simulated_tx_list);
    }

    /// Removes a transaction with sender and nonce from the pool.
    pub fn remove(&mut self, sender: &Address, nonce: u64) {
        if let Some(tx_list) = self.pool_data.get_mut(sender) {
            if tx_list.forward(&nonce) {
                self.pool_data.remove(sender);
            }
        }

        self.active_txs.forward(sender, nonce);
    }

    /// Clears all mined txs from the pending list and resets the active list.
    /// This gets called in two places:
    /// 1) When we sync a new block.
    /// 2) When we commit a new Frag.
    pub fn handle_new_mined_txs<'a, Db: DatabaseRead, I, T: TransactionSenderInfo + 'a>(
        &mut self,
        mined_txs: I,
        base_fee: u64,
        db: &DBFrag<Db>,
        syncing: bool,
        sim_sender: Option<&SendersSpine<Db>>,
    ) where
        I: IntoIterator<Item = &'a T>,
        I::IntoIter: DoubleEndedIterator,
    {
        // Clear all mined nonces from the pool
        for tx in mined_txs.into_iter().rev() {
            let sender = tx.sender();
            if let Some(sender_tx_list) = self.pool_data.get_mut(&sender) {
                if sender_tx_list.forward(&tx.nonce()) {
                    self.pool_data.remove(&sender);
                }
            }
        }

        // Completely wipe active txs as they may contain valid nonces with out of date sim results.
        self.active_txs.clear();

        // If enabled, fill the active list with non-simulated txs and send off the first tx for each sender to
        // simulator.
        if !syncing {
            for (sender, tx_list) in self.pool_data.iter() {
                let db_nonce = db.get_nonce(*sender).unwrap();
                if let Some(ready) = tx_list.ready(db_nonce, base_fee) {
                    TxPool::send_sim_requests_for_tx(ready.peek().unwrap(), db, sim_sender);
                    self.active_txs.put(SimulatedTxList::new(None, tx_list));
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
