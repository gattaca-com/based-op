//! Bundle order type definitions and related functionality.

use std::{
    hash::{Hash, Hasher},
    sync::OnceLock,
};

use alloy_eips::{Decodable2718, Encodable2718, eip2718::Eip2718Error};
use alloy_primitives::{B256, Bytes, TxHash, U64};
use alloy_rpc_types::mev::EthSendBundle;
use op_alloy_consensus::OpTxEnvelope;
use reth_primitives_traits::SignedTransaction;

/// Type alias for a validated bundle.
pub type ValidatedBundle = Bundle<Transaction>;

use super::Transaction;

/// An internal, minimal bundle type.
#[derive(Debug)]
pub struct Bundle<T> {
    pub block_number: U64,
    pub transactions: Vec<T>,
    pub reverting_tx_hashes: Option<Vec<TxHash>>,

    // Cached bundle hash that's initialized on first use.
    bundle_hash: OnceLock<B256>,
}

impl From<EthSendBundle> for Bundle<Bytes> {
    fn from(bundle: EthSendBundle) -> Self {
        let reverting_tx_hashes =
            if bundle.reverting_tx_hashes.is_empty() { None } else { Some(bundle.reverting_tx_hashes) };

        Self {
            block_number: U64::from(bundle.block_number),
            transactions: bundle.txs,
            reverting_tx_hashes,
            bundle_hash: OnceLock::new(),
        }
    }
}

impl Hash for Bundle<Bytes> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.block_number.hash(state);
        self.transactions.hash(state);
        // FIXME: This is actually not fully compatible with <https://docs.titanbuilder.xyz/api/eth_sendbundle#bundle-hash>,
        // because they use strings for the reverting tx hashes.
        self.reverting_tx_hashes.hash(state);
    }
}

impl Bundle<Bytes> {
    /// Calculates the bundle hash similarly to <https://docs.titanbuilder.xyz/api/eth_sendbundle#bundle-hash>,
    /// but using only the supported fields.
    pub fn bundle_hash(&self) -> B256 {
        *self.bundle_hash.get_or_init(|| {
            let mut hasher = wyhash::WyHash::default();
            let mut bytes = [0u8; 32];
            for i in 0..4 {
                self.hash(&mut hasher);
                let hash = hasher.finish();
                bytes[(i * 8)..((i + 1) * 8)].copy_from_slice(&hash.to_be_bytes());
            }

            B256::from(bytes)
        })
    }

    /// Tries to decode the RLP-encoded transactions into a bundle of [`OpTxEnvelope`]s.
    pub fn try_decode(self) -> Result<Bundle<OpTxEnvelope>, BundleValidationError> {
        // Ensure the bundle hash is initialized before converting.
        let _ = self.bundle_hash();
        let transactions = self
            .transactions
            .into_iter()
            .map(|tx| OpTxEnvelope::decode_2718(&mut tx.as_ref()))
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Bundle {
            block_number: self.block_number,
            transactions,
            reverting_tx_hashes: self.reverting_tx_hashes,
            bundle_hash: self.bundle_hash,
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum BundleValidationError {
    #[error("invalid transaction encoding: {0:?}")]
    DecodeError(#[from] Eip2718Error),
    #[error("invalid signature on transaction: {0:?}")]
    InvalidSignature(TxHash),
}

impl Bundle<OpTxEnvelope> {
    /// Returns the bundle hash of the bundle.
    pub fn bundle_hash(&self) -> B256 {
        // SAFETY: At this point, the bundle hash is guaranteed to be initialized.
        *self.bundle_hash.get().expect("bundle hash is not initialized")
    }

    /// Validates the bundle, including signature validation of included transactions.
    ///
    /// This is a CPU-intensive operation.
    pub fn validate(self) -> Result<ValidatedBundle, BundleValidationError> {
        let recovered = self
            .transactions
            .into_iter()
            .map(|tx| {
                let recovered =
                    tx.try_into_recovered().map_err(|tx| BundleValidationError::InvalidSignature(tx.tx_hash()))?;
                let (tx, signer) = recovered.into_parts();
                let encoded = tx.encoded_2718();
                Ok::<_, BundleValidationError>(Transaction::new(tx, signer, encoded.into()))
            })
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Bundle {
            block_number: self.block_number,
            transactions: recovered,
            reverting_tx_hashes: self.reverting_tx_hashes,
            bundle_hash: self.bundle_hash,
        })
    }
}
