use std::{collections::VecDeque, sync::Arc};

use bop_common::{
    communication::{
        messages::{SequencerToSimulator, SimulationResult},
        SpineConnections,
    },
    db::DatabaseRead,
    time::Instant,
    transaction::{SimulatedTx, Transaction},
};
use revm_primitives::Address;
use tracing::error;

use crate::sorting::{ActiveOrders, InSortFrag};

/// State of the sequencer while sorting frags
#[derive(Clone, Debug)]
pub struct SortingData<Db: DatabaseRead> {
    /// Current frag being sorted
    pub frag: InSortFrag<Db>,
    /// Deadline when to seal the current frag
    pub until: Instant,
    /// How many simulations we are waiting for
    pub in_flight_sims: usize,
    /// All orders simulated on top of the current frag
    pub tof_snapshot: ActiveOrders,
    /// Next best order to apply
    pub next_to_be_applied: Option<SimulatedTx>,
}

impl<Db: DatabaseRead> SortingData<Db> {
    pub fn apply_and_send_next(
        mut self,
        n_sims_per_loop: usize,
        senders: &mut SpineConnections<Db>,
        base_fee: u64,
    ) -> Self {
        self.maybe_apply(base_fee);

        let db = self.frag.state();

        for t in self.tof_snapshot.iter().rev().take(n_sims_per_loop).map(|t| t.next_to_sim()) {
            debug_assert!(t.is_some(), "Unsimmable TxList should have been cleared previously");
            let tx = t.unwrap();
            senders.send(SequencerToSimulator::SimulateTx(tx, db.clone()));
            self.in_flight_sims += 1;
        }
        self
    }

    pub fn maybe_apply(&mut self, base_fee: u64) {
        if let Some(tx_to_apply) = std::mem::take(&mut self.next_to_be_applied) {
            self.tof_snapshot.remove_from_sender(&tx_to_apply.sender(), base_fee);
            self.frag.apply_tx(tx_to_apply);
        }
    }

    pub fn is_valid(&self, state_id: u64) -> bool {
        state_id == self.frag.db.state_id()
    }

    /// Handles the result of a simulation. `simulated_tx` simulated_at_id should be pre-verified.
    pub fn handle_sim(&mut self, simulated_tx: SimulationResult<SimulatedTx>, sender: &Address, base_fee: u64) {
        self.in_flight_sims -= 1;

        // handle errored sim
        let Ok(simulated_tx) = simulated_tx.inspect_err(|e| error!("simming tx for sender {sender} {e}",)) else {
            self.tof_snapshot.remove_from_sender(sender, base_fee);
            return;
        };

        let tx_to_put_back = if self.next_to_be_applied.as_ref().is_none_or(|t| t.payment < simulated_tx.payment) {
            self.next_to_be_applied.replace(simulated_tx)
        } else {
            Some(simulated_tx)
        };
        if let Some(tx) = tx_to_put_back {
            self.tof_snapshot.put(tx)
        }
    }

    pub fn should_seal_frag(&self) -> bool {
        // for now this is to get around the fact that if the initialy opattrs contains a list of txs to be included and
        // we sent off the last one to be included while the self.until runs out, it would return true. What we
        // should do is keep track whether we're the first frag and not return ever until remaining attributes
        // txs == 0 and num in flights == 0
        if self.in_flight_sims != 0 {
            return false;
        }
        (self.tof_snapshot.is_empty() || self.until < Instant::now())
    }

    pub fn should_send_next_sims(&self) -> bool {
        self.in_flight_sims == 0
    }

    pub fn is_empty(&self) -> bool {
        self.frag.is_empty()
    }
}
