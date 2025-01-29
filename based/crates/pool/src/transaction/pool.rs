use std::{collections::HashMap, sync::Arc};

use alloy_primitives::Address;
use bop_common::{
    communication::{messages::SequencerToSimulator, sequencer::SendersSequencer, TrackedSenders},
    time::Duration,
    transaction::{SimulatedTxList, Transaction, TxList},
};
use revm_primitives::db::DatabaseRef;

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

    pub fn handle_new_tx<Db>(
        &mut self,
        new_tx: Arc<Transaction>,
        db: &Db,
        base_fee: u64,
        sim_sender: &SendersSequencer<Db>,
    ) where
        Db: DatabaseRef,
        <Db as DatabaseRef>::Error: std::fmt::Debug,
    {
        let mut state_nonce = get_nonce(db, new_tx.sender());

        //TODO: remove
        debug_assert!(
            sim_sender
                .send_timeout(
                    SequencerToSimulator::SimulateTxList(None, vec![new_tx.clone()]),
                    Duration::from_millis(10)
                )
                .is_ok(),
            "Couldn't send simulator reply"
        );

        // check nonce is valid
        if new_tx.nonce() < state_nonce {
            return;
        }

        // Add to pool and send to simulator if mineable
        match self.pool_data.get_mut(new_tx.sender_ref()) {
            Some(tx_list) => {
                // If it conflicts with a current tx compare effective gas prices, this also
                // overwrites if gas price is equal, taking into account conditions
                // above where we didn't return
                if tx_list.get_effective_price_for_nonce(new_tx.nonce_ref(), base_fee) >
                    new_tx.effective_gas_price(base_fee)
                {
                    return;
                }

                tx_list.put(new_tx);
                if let Some(mineable_txs) = tx_list.ready(&mut state_nonce, base_fee) {
                    debug_assert!(
                        sim_sender
                            .send_timeout(
                                SequencerToSimulator::SimulateTxList(None, mineable_txs),
                                Duration::from_millis(10)
                            )
                            .is_ok(),
                        "Couldn't send simulator reply"
                    );
                }
            }
            None => {
                let tx_list = TxList::from(new_tx);
                if let Some(mineable_txs) = tx_list.ready(&mut state_nonce, base_fee) {
                    self.send_sim_requests_for_txs(mineable_txs);
                }

                self.pool_data.insert(tx_list.sender(), tx_list);
            }
        }
    }

    /// Process all simulated mineable txs in queue.
    /// Simulation requests are grouped by sender.
    /// Add all successful simulations to a SortTxList and overwite any current list for that
    /// sender.
    pub fn handle_simulated(&mut self) {
        // TODO: receive sim results, add to active_txs if successful
    }

    #[allow(unused)]
    fn handle_new_block<Db>(&mut self, mined_txs: &[Arc<Transaction>], base_fee: u64, db: &Db)
    where
        Db: DatabaseRef,
        <Db as DatabaseRef>::Error: std::fmt::Debug,
    {
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

        // Send mineable txs for each active sender to simulator
        for (sender, sender_txs) in self.pool_data.iter() {
            let mut expected_nonce = get_nonce(&db, *sender);
            if let Some(txs) = sender_txs.ready(&mut expected_nonce, base_fee) {
                self.send_sim_requests_for_txs(txs);
            }
        }
    }

    pub fn clone_active(&self) -> Vec<SimulatedTxList> {
        self.active_txs.clone_txs()
    }

    pub fn active(&self) -> &[SimulatedTxList] {
        self.active_txs.txs()
    }

    fn send_sim_requests_for_txs(&self, _txs: Vec<Arc<Transaction>>) {
        todo!()
    }

    pub fn num_active_txs(&self) -> usize {
        self.active_txs.num_txs()
    }

    pub fn active_empty(&self) -> bool {
        self.active_txs.is_empty()
    }
}

#[inline]
fn get_nonce<Db>(db: &Db, address: Address) -> u64
where
    Db: DatabaseRef,
    <Db as DatabaseRef>::Error: std::fmt::Debug,
{
    db.basic_ref(address).unwrap().map_or(0, |acc| acc.nonce)
}
