use std::sync::Arc;

use alloy_consensus::{
    proofs::{calculate_transaction_root, ordered_trie_root_with_encoder},
    Header, EMPTY_OMMER_ROOT_HASH,
};
use alloy_eips::{eip2718::Encodable2718, merge::BEACON_NONCE};
use alloy_primitives::{Bloom, U256};
use alloy_rpc_types::{
    engine::{BlobsBundleV1, ExecutionPayloadV1, ExecutionPayloadV2, ExecutionPayloadV3},
    logs_bloom,
};
use bop_common::{
    db::{flatten_state_changes, DBFrag, DBSorting},
    p2p::{FragV0, SealV0, VersionedMessage},
    transaction::SimulatedTx,
};
use bop_db::BopDbRead;
use op_alloy_rpc_types_engine::OpExecutionPayloadEnvelopeV3;
use reth_evm::NextBlockEnvAttributes;
use reth_optimism_consensus::calculate_receipt_root_no_memo_optimism;
use revm_primitives::{Address, BlockEnv, Bytes, B256};

/// Sequence of frags applied on the last block
#[derive(Clone, Debug)]
pub struct FragSequence<Db> {
    db: DBFrag<Db>,
    gas_remaining: u64,
    payment: U256,
    txs: Vec<SimulatedTx>,
    /// Next frag index
    next_seq: u64,
    /// Block number for all frags in this block
    block_number: u64,
}

impl<Db: BopDbRead + Clone + std::fmt::Debug> FragSequence<Db> {
    pub fn new(db: DBFrag<Db>, max_gas: u64) -> Self {
        let block_number = db.head_block_number().expect("can't get block number") + 1;
        Self { db, gas_remaining: max_gas, payment: U256::ZERO, txs: vec![], next_seq: 0, block_number }
    }

    pub fn db(&self) -> DBFrag<Db> {
        self.db.clone()
    }

    pub fn db_ref(&self) -> &DBFrag<Db> {
        &self.db
    }

    /// Builds a new in-sort frag
    pub fn create_in_sort(&self) -> InSortFrag<Db> {
        let db_sort = DBSorting::new(self.db());
        InSortFrag::new(db_sort, self.gas_remaining)
    }

    /// Creates a new frag, all subsequent frags will be built on top of this one
    pub fn apply_sorted_frag(&mut self, in_sort: InSortFrag<Db>) -> FragV0 {
        self.gas_remaining -= in_sort.gas_used;
        self.payment += in_sort.payment;

        let msg = FragV0::new(self.block_number, self.next_seq, in_sort.txs.iter().map(|tx| tx.tx.as_ref()), false);

        self.db.commit(in_sort.txs.iter());
        self.txs.extend(in_sort.txs);
        self.next_seq += 1;

        msg
    }

    /// When a new block is received, we clear all the temp state on the db
    pub fn clear_frags(&mut self) {
        self.db.reset();
    }

    pub fn seal_block(
        &self,
        block_env: &BlockEnv,
        chain_spec: impl reth_chainspec::Hardforks,
        parent_hash: B256,
    ) -> (SealV0, OpExecutionPayloadEnvelopeV3) {
        let state_changes = flatten_state_changes(self.txs.iter().map(|t| t.result_and_state.state.clone()).collect());
        let state_root = self.db.state_root(state_changes);

        let mut receipts = vec![];
        let mut transactions = vec![];
        let mut logs_bloom = Bloom::ZERO;
        let mut gas_used = 0;

        for t in self.txs.iter() {
            let receipt = t.receipt(gas_used);
            logs_bloom |= receipt.logs_bloom;
            receipts.push(receipt);
            transactions.push(t.tx.encode());
            gas_used += t.result_and_state.result.gas_used();
        }

        let receipts_root = ordered_trie_root_with_encoder(&receipts, |r, buf| {
            r.encode_2718(buf);
        });

        let transactions_root = ordered_trie_root_with_encoder(&self.txs, |tx, buf| tx.encode_2718(buf));
        let header = Header {
            parent_hash,
            ommers_hash: EMPTY_OMMER_ROOT_HASH,
            beneficiary: block_env.coinbase,
            state_root,
            transactions_root,
            receipts_root,
            withdrawals_root: None,
            logs_bloom,
            timestamp: block_env.timestamp.to(),
            mix_hash: block_env.prevrandao.unwrap_or_default(),
            nonce: BEACON_NONCE.into(),
            base_fee_per_gas: Some(block_env.basefee.to()),
            number: block_env.number.to(),
            gas_limit: block_env.gas_limit.to(),
            difficulty: U256::ZERO,
            gas_used,
            extra_data: Bytes::default(),
            parent_beacon_block_root: None,
            blob_gas_used: None,
            excess_blob_gas: None,
            requests_hash: None,
        };

        let v1 = ExecutionPayloadV1 {
            parent_hash,
            fee_recipient: block_env.coinbase,
            state_root,
            receipts_root,
            logs_bloom,
            prev_randao: block_env.prevrandao.unwrap_or_default(),
            block_number: block_env.number.to(),
            gas_limit: block_env.gas_limit.to(),
            gas_used,
            timestamp: block_env.timestamp.to(),
            extra_data: Bytes::default(),
            base_fee_per_gas: block_env.basefee,
            block_hash: header.hash_slow(),
            transactions,
        };
        (
            SealV0 {
                total_frags: self.next_seq,
                block_number: block_env.number.to(),
                gas_used,
                gas_limit: block_env.gas_limit.to(),
                parent_hash,
                transactions_root,
                receipts_root,
                state_root,
                block_hash: v1.block_hash,
            },
            OpExecutionPayloadEnvelopeV3 {
                execution_payload: ExecutionPayloadV3 {
                    payload_inner: ExecutionPayloadV2 { payload_inner: v1, withdrawals: vec![] },
                    blob_gas_used: 0,
                    excess_blob_gas: 0,
                },
                block_value: self.payment,
                blobs_bundle: BlobsBundleV1::new(vec![]),
                should_override_builder: false,
                parent_beacon_block_root: B256::ZERO,
            },
        )
    }

    pub fn is_valid(&self, state_id: u64) -> bool {
        state_id == self.db.state_id()
    }
}

/// Fragment of a block being sorted and built
#[derive(Clone, Debug)]
pub struct InSortFrag<Db> {
    pub db: Arc<DBSorting<Db>>,
    pub gas_remaining: u64,
    pub gas_used: u64,
    pub payment: U256,
    pub txs: Vec<SimulatedTx>,
}

impl<Db: std::fmt::Debug + Clone> InSortFrag<Db> {
    pub fn new(db: DBSorting<Db>, max_gas: u64) -> Self {
        Self { db: Arc::new(db), gas_remaining: max_gas, gas_used: 0, payment: U256::ZERO, txs: vec![] }
    }

    pub fn apply_tx(&mut self, mut tx: SimulatedTx) {
        let db = Arc::make_mut(&mut self.db);
        db.commit(tx.take_state());
        self.payment += tx.payment;

        // TODO: check gas usage
        let gas_used = tx.as_ref().result.gas_used();
        debug_assert!(
            self.gas_remaining > gas_used,
            "had too little gas remaining on block {self:#?} to apply tx {tx:#?}"
        );

        self.gas_remaining -= gas_used;
        self.gas_used += gas_used;
        self.txs.push(tx);
    }

    pub fn state(&self) -> Arc<DBSorting<Db>> {
        self.db.clone()
    }
}
