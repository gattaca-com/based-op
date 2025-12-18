use alloy_consensus::{Header, TxEnvelope};
use alloy_eips::eip2718::Decodable2718;
use alloy_primitives::{B256, Bytes};
use alloy_rpc_types::{Log, TransactionReceipt, state::StateOverride};
use bop_common::p2p::{EnvV0, FragV0, Transaction as TxBytes};
use op_alloy_consensus::OpReceiptEnvelope;
use reth_revm::db::Cache;

use crate::error::UnsealedBlockError;

#[derive(Debug, Clone)]
pub struct UnsealedBlock {
    /// Block environment.
    pub env: EnvV0,
    /// Received fragments that contain the raw transaction bytes.
    pub frags: Vec<FragV0>,
    /// Sequence number of the last fragment that has been accepted/added.
    ///
    /// - `None` means no fragment has been accepted yet
    /// - `Some(n)` means `frags[n]` is considered the latest known fragment
    pub last_sequence_number: Option<u64>,
    /// Block hash.
    pub hash: B256,

    /// Transaction receipts for executed transactions.
    pub receipts: Vec<TransactionReceipt<OpReceiptEnvelope<Log>>>,
    /// Flattened logs emitted during execution.
    pub logs: Vec<Log>,
    /// Cumulative execution gas used across all transactions in the block.
    pub cumulative_gas_used: u64,
    /// Cumulative blob gas used across all blob-carrying transactions in the block.
    pub cumulative_blob_gas_used: u64,

    db_cache: Cache,
    state_overrides: Option<StateOverride>,
}

impl UnsealedBlock {
    pub fn new(env: EnvV0) -> Self {
        Self {
            env,
            frags: Vec::new(),
            last_sequence_number: None,
            hash: Default::default(),
            receipts: Vec::new(),
            logs: Vec::new(),
            cumulative_gas_used: 0,
            cumulative_blob_gas_used: 0,
            db_cache: Cache::default(),
            state_overrides: None,
        }
    }

    /// Returns `true` if no fragments have been added yet.
    pub fn is_empty(&self) -> bool {
        self.frags.is_empty()
    }

    /// Returns `true` if `f` is the next fragment that should be appended.
    pub fn is_next_frag(&self, f: &FragV0) -> bool {
        match self.last_sequence_number {
            None => f.is_first(),
            Some(last_seq) => {
                let Some(last_known) = self.frags.get(last_seq as usize) else {
                    return false;
                };
                if last_known.is_last { false } else { last_known.seq + 1 == f.seq }
            }
        }
    }

    /// Raw tx bytes iterator (flattening frags)
    pub fn transactions_iter_bytes(&self) -> impl Iterator<Item = &TxBytes> + '_ {
        self.frags.iter().flat_map(|frag| frag.txs.iter())
    }

    /// Decoded txs iterator (lazy decode)
    pub fn transactions_iter_decoded(&self) -> impl Iterator<Item = Result<TxEnvelope, UnsealedBlockError>> + '_ {
        self.transactions_iter_bytes().enumerate().map(|(index, tx)| {
            // allocate a Vec<u8> to decode from
            let raw: Vec<u8> = tx.iter().copied().collect();
            TxEnvelope::decode_2718_exact(&raw).map_err(|source| UnsealedBlockError::TxDecode { index, source })
        })
    }

    /// Decoded txs (allocates Vec), like Go `Transactions()` but decoded
    pub fn transactions(&self) -> Result<Vec<TxEnvelope>, UnsealedBlockError> {
        self.transactions_iter_decoded().collect()
    }

    /// Raw tx bytes (allocates Vec<Vec<u8>>), like Go `ByteTransactions()`
    pub fn byte_transactions(&self) -> Vec<Vec<u8>> {
        self.transactions_iter_bytes().map(|tx| tx.iter().copied().collect::<Vec<u8>>()).collect()
    }

    // Return the last frag on the list.
    pub fn last_frag(&self) -> Option<&FragV0> {
        self.frags.last()
    }

    /// Apply the accepted frag into in-memory bookkeeping (NOT executing txs).
    ///
    /// Execution results (receipts/logs/gas) should be appended separately.
    pub fn accept_frag_execution(
        &mut self,
        f: FragV0,
        logs: Vec<Log>,
        receipts: Vec<TransactionReceipt<OpReceiptEnvelope<Log>>>,
        cummulative_gas_used: u64,
    ) {
        self.last_sequence_number = Some(f.seq);
        self.cumulative_blob_gas_used = self.cumulative_blob_gas_used.saturating_add(f.blob_gas_used);
        self.frags.push(f.clone());
        self.logs.extend_from_slice(logs.as_slice());
        self.receipts.extend_from_slice(receipts.as_slice());
        self.cumulative_gas_used = cummulative_gas_used;
    }

    /// Validate frag against current state (equivalent to your ValidateNewFragV0 + sequencing gate).
    pub fn validate_new_frag(&self, f: &FragV0) -> Result<(), UnsealedBlockError> {
        if f.block_number < self.env.number {
            return Err(UnsealedBlockError::StaleFrag { frag_block: f.block_number, env_number: self.env.number });
        }

        // must target current block
        if f.block_number > self.env.number {
            return Err(UnsealedBlockError::WrongBlock { frag_block: f.block_number, env_number: self.env.number });
        }

        // sequencing
        if !self.is_next_frag(f) {
            let last = self.last_sequence_number;
            if self.frags.last().is_some_and(|x| x.is_last) {
                return Err(UnsealedBlockError::AlreadyEnded);
            }
            return Err(UnsealedBlockError::SeqMismatch { got: f.seq, last });
        }

        Ok(())
    }

    /// A temporary header derived from env.
    pub fn temp_header(&self) -> Header {
        Header {
            parent_hash: self.env.parent_hash,
            parent_beacon_block_root: Some(self.env.parent_beacon_block_root),
            number: self.env.number,
            timestamp: self.env.timestamp,
            extra_data: Bytes::copy_from_slice(self.env.extra_data.as_ref()),
            gas_limit: self.env.gas_limit,
            base_fee_per_gas: Some(self.env.basefee),
            difficulty: self.env.difficulty,
            beneficiary: self.env.beneficiary,
            mix_hash: self.env.prevrandao,

            // placeholders until seal-time
            ommers_hash: B256::ZERO,
            state_root: B256::ZERO,
            transactions_root: B256::ZERO,
            receipts_root: B256::ZERO,
            logs_bloom: Default::default(),
            gas_used: 0,
            nonce: Default::default(),
            withdrawals_root: None,
            blob_gas_used: None,
            excess_blob_gas: None,
            requests_hash: None,
        }
    }

    /// Reset to a fresh env (drop frags/results/counters).
    pub fn reset_to_env(&mut self, env: EnvV0) {
        *self = Self::new(env);
    }

    pub fn with_db_cache(mut self, cache: Cache) -> Self {
        self.db_cache = cache;
        self
    }

    pub fn with_state_overrides(mut self, state_overrides: Option<StateOverride>) -> Self {
        self.state_overrides = state_overrides;
        self
    }

    /// Returns the database cache.
    pub fn get_db_cache(&self) -> Cache {
        self.db_cache.clone()
    }

    /// Returns the state overrides for the pending state.
    pub fn get_state_overrides(&self) -> Option<StateOverride> {
        self.state_overrides.clone()
    }

    pub fn clone_for_update(&self) -> Self {
        Self {
            env: self.env.clone(),
            frags: self.frags.clone(),
            last_sequence_number: self.last_sequence_number,
            hash: self.hash,
            receipts: self.receipts.clone(),
            logs: self.logs.clone(),
            cumulative_gas_used: self.cumulative_gas_used,
            cumulative_blob_gas_used: self.cumulative_blob_gas_used,
            db_cache: self.db_cache.clone(),
            state_overrides: self.state_overrides.clone(),
        }
    }
}
