use alloy_consensus::{Header, TxEnvelope};
use alloy_eips::eip2718::{Decodable2718, Eip2718Error};
use alloy_primitives::{B256, Bytes, FixedBytes};
use alloy_rpc_types::{Log, TransactionReceipt};
use bop_common::p2p::{EnvV0, FragV0, Transaction as TxBytes};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum UnsealedBlockError {
    #[error("failed to decode EIP-2718 tx at index {index}")]
    TxDecode {
        index: usize,
        #[source]
        source: Eip2718Error,
    },
}

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
    pub hash: FixedBytes<32>,

    /// Transaction receipts for executed transactions.
    pub receipts: Vec<TransactionReceipt>,
    /// Flattened logs emitted during execution.
    pub logs: Vec<Log>,
    /// Cumulative execution gas used across all transactions in the block.
    pub cumulative_gas_used: u64,
    /// Cumulative blob gas used across all blob-carrying transactions in the block.
    pub cumulative_blob_gas_used: u64,
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
                if last_known.is_last {
                    false
                } else {
                    last_known.seq + 1 == f.seq
                }
            }
        }
    }

    /// Raw tx bytes iterator (flattening frags)
    pub fn transactions_iter_bytes(&self) -> impl Iterator<Item = &TxBytes> + '_ {
        self.frags.iter().flat_map(|frag| frag.txs.iter())
    }

    /// Decoded txs iterator (lazy decode)
    pub fn transactions_iter_decoded(
        &self,
    ) -> impl Iterator<Item = Result<TxEnvelope, UnsealedBlockError>> + '_ {
        self.transactions_iter_bytes()
            .enumerate()
            .map(|(index, tx)| {
                // allocate a Vec<u8> to decode from
                let raw: Vec<u8> = tx.iter().copied().collect();
                TxEnvelope::decode_2718_exact(&raw)
                    .map_err(|source| UnsealedBlockError::TxDecode { index, source })
            })
    }

    /// Decoded txs (allocates Vec), like Go `Transactions()` but decoded
    pub fn transactions(&self) -> Result<Vec<TxEnvelope>, UnsealedBlockError> {
        self.transactions_iter_decoded().collect()
    }

    /// Raw tx bytes (allocates Vec<Vec<u8>>), like Go `ByteTransactions()`
    pub fn byte_transactions(&self) -> Vec<Vec<u8>> {
        self.transactions_iter_bytes()
            .map(|tx| tx.iter().copied().collect::<Vec<u8>>())
            .collect()
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
}
