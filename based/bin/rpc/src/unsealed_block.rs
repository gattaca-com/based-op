use std::collections::VecDeque;

use alloy_network::ReceiptResponse;
use alloy_primitives::{Address, B256, U256, map::foldhash::HashMap};
use bop_common::p2p::{DetailedStateChange, EnvV0, FragV0, SealV0, StateUpdate};
use op_alloy_rpc_types::OpTransactionReceipt;
use tracing::error;

pub struct UnsealedBlock {
    pub env: EnvV0,
    pub current_frag: Option<FragV0>,
    pub transaction_count_diff: HashMap<Address, u64>,
    pub receipts: HashMap<B256, OpTransactionReceipt>,
    pub balances: HashMap<Address, U256>,
    pub state_changes: HashMap<Address, DetailedStateChange>,
    pub seal: Option<SealV0>,
}

impl UnsealedBlock {
    pub fn apply_frag(&mut self, frag: FragV0, state_update: Option<StateUpdate>) {
        if self.current_frag.is_none() {
            if frag.seq != 0 {
                error!("expected first frag to have seq 0 but got seq {}", frag.seq);
                return;
            }
        } else {
            let current_frag = self.current_frag.as_ref().unwrap();
            let expected_seq = current_frag.seq + 1;
            if expected_seq != frag.seq {
                error!("expected frag seq {} but got seq {}", expected_seq, frag.seq);
                return;
            }
        }
        if self.seal.is_some() {
            error!("trying to apply frag after seal");
            return;
        }

        self.current_frag = Some(frag);

        if let Some(state_update) = state_update {
            for (_tx_hash, receipt) in state_update.receipts.iter() {
                let sender = receipt.from();
                self.transaction_count_diff.entry(sender).and_modify(|count| *count += 1).or_insert(1);
            }
            self.receipts.extend(state_update.receipts);
            self.balances.extend(state_update.balances);

            if let Some(state_changes) = state_update.state_changes {
                for (address, state_change) in state_changes.iter() {
                    let account = self.state_changes.entry(*address).or_default();
                    account.balance = state_change.balance;
                    account.nonce = state_change.nonce;
                    account.storage.extend(state_change.storage.iter());
                }
            }
        }
    }

    pub fn apply_seal(&mut self, seal: SealV0) {
        self.seal = Some(seal);
    }

    pub fn get_transaction_count_diff(&self, address: Address) -> Option<u64> {
        self.transaction_count_diff.get(&address).cloned()
    }

    pub fn get_receipt(&self, tx_hash: B256) -> Option<OpTransactionReceipt> {
        self.receipts.get(&tx_hash).cloned()
    }

    pub fn get_balance(&self, address: Address) -> Option<U256> {
        self.balances.get(&address).cloned()
    }

    pub fn get_state_changes(&self) -> HashMap<Address, DetailedStateChange> {
        self.state_changes.clone()
    }
}

pub struct UnsealedBlockStack {
    pub blocks: VecDeque<UnsealedBlock>,
    pub root_provider_block_number: Option<u64>,
}

impl UnsealedBlockStack {
    pub fn new() -> Self {
        Self { blocks: VecDeque::new(), root_provider_block_number: None }
    }

    pub fn get_transaction_count_diff(&self, address: Address) -> u64 {
        let mut total_diff = 0;
        for block in self.blocks.iter().rev() {
            total_diff += block.get_transaction_count_diff(address).unwrap_or(0);
        }
        total_diff
    }

    pub fn get_receipt(&self, tx_hash: B256) -> Option<OpTransactionReceipt> {
        for block in self.blocks.iter().rev() {
            if let Some(receipt) = block.get_receipt(tx_hash) {
                return Some(receipt);
            }
        }
        None
    }

    pub fn get_balance(&self, address: Address) -> Option<U256> {
        for block in self.blocks.iter().rev() {
            if let Some(balance) = block.get_balance(address) {
                return Some(balance);
            }
        }
        None
    }

    pub fn block_number(&self) -> Option<u64> {
        if let Some(block) = self.blocks.back() {
            return Some(block.env.number);
        }
        if let Some(root_provider_block_number) = self.root_provider_block_number {
            return Some(root_provider_block_number);
        }
        None
    }

    pub fn get_state_changes(&self) -> HashMap<Address, DetailedStateChange> {
        let mut state_changes = HashMap::default();
        for block in self.blocks.iter() {
            for (address, state_change) in block.get_state_changes().iter() {
                let account = state_changes.entry(*address).or_insert_with(DetailedStateChange::default);
                account.balance = state_change.balance;
                account.nonce = state_change.nonce;
                account.storage.extend(state_change.storage.iter());
            }
        }
        state_changes
    }
}
