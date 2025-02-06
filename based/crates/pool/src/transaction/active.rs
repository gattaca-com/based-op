use alloy_consensus::Transaction;
use alloy_primitives::Address;
use bop_common::transaction::SimulatedTxList;
use rustc_hash::FxHashMap;

#[derive(Debug, Clone, Default)]
pub struct Active {
    pub txs: Vec<SimulatedTxList>,
    /// These are the senders that we have txs for in the active list.
    /// Maps sender to index in `txs`.
    senders: FxHashMap<Address, usize>,
}

impl Active {
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            txs: Vec::with_capacity(capacity),
            senders: FxHashMap::with_capacity_and_hasher(capacity, Default::default()),
        }
    }

    #[inline]
    pub fn put(&mut self, tx: SimulatedTxList) {
        let sender = tx.sender();

        if let Some(&index) = self.senders.get(&sender) {
            self.txs[index] = tx;
        } else {
            self.txs.push(tx);
            self.senders.insert(sender, self.txs.len() - 1);
        }
    }

    #[inline]
    pub fn clear(&mut self) {
        self.senders.clear();
        self.txs.clear();
    }

    #[inline]
    pub fn clone_txs(&self) -> Vec<SimulatedTxList> {
        self.txs.clone()
    }

    #[inline]
    pub fn txs(&self) -> &[SimulatedTxList] {
        &self.txs
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.txs.is_empty()
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.txs.len()
    }

    /// Returns the total number of individual transactions in the active list.
    #[inline]
    pub fn num_txs(&self) -> usize {
        self.txs.iter().map(|tx| tx.len()).sum()
    }

    #[inline]
    pub fn tx_list_mut(&mut self, sender: &Address) -> Option<&mut SimulatedTxList> {
        self.senders.get_mut(sender).map(|index| &mut self.txs[*index])
    }

    #[inline]
    pub fn forward(&mut self, address: &Address, nonce: u64) {
        let Some(&index) = self.senders.get(address) else {
            return;
        };

        let tx_list = &mut self.txs[index];
        if tx_list.pending.forward(&nonce) {
            self.remove_index(index);
            return;
        }

        if let Some(ref current) = tx_list.current {
            if nonce >= current.nonce() {
                tx_list.current = None;
            }
        }
    }

    #[inline]
    fn remove_index(&mut self, index: usize) {
        let sender = self.txs[index].sender();

        // Remove the sender from the active list.
        self.txs.swap_remove(index);
        self.senders.remove(&sender);

        // If we swapped with a tx (wasn't the last element), update its sender's index.
        if index < self.txs.len() {
            let swapped_sender = self.txs[index].sender();
            *self.senders.get_mut(&swapped_sender).unwrap() = index;
        }
    }
}
