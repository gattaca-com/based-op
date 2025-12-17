//! Bundle order type definitions and related functionality.

use std::{
    hash::{Hash, Hasher},
    sync::{Arc, OnceLock},
};

use alloy_consensus::Transaction as _;
use alloy_eips::{Decodable2718, Encodable2718, eip2718::Eip2718Error};
use alloy_primitives::{Address, B256, Bytes, TxHash, U64, U256};
use alloy_rpc_types::mev::EthSendBundle;
use op_alloy_consensus::OpTxEnvelope;
use reth_primitives_traits::{InMemorySize, SignedTransaction};
use uuid::Uuid;

use crate::{
    order::ResultAndStateMany,
    time::Nanos,
    transaction::{SimulatedTx, Transaction},
};

/// Type alias for a validated bundle.
pub type ValidatedBundle = Bundle<Transaction>;

/// An internal, minimal bundle type.
#[derive(Debug)]
pub struct Bundle<T> {
    pub id: Uuid,
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
            id: Uuid::new_v4(),
            block_number: U64::from(bundle.block_number),
            transactions: bundle.txs,
            reverting_tx_hashes,
            bundle_hash: OnceLock::new(),
        }
    }
}

impl<T> Bundle<T> {
    pub fn id(&self) -> Uuid {
        self.id
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
            id: self.id,
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
        let transactions = self
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
            id: self.id,
            block_number: self.block_number,
            transactions,
            reverting_tx_hashes: self.reverting_tx_hashes,
            bundle_hash: self.bundle_hash,
        })
    }
}

impl InMemorySize for ValidatedBundle {
    fn size(&self) -> usize {
        self.transactions.iter().map(|tx| tx.size()).sum()
    }
}

impl InMemorySize for Bundle<SimulatedTx> {
    fn size(&self) -> usize {
        self.transactions.iter().map(|tx| tx.size()).sum()
    }
}

impl ValidatedBundle {
    pub fn uuid(&self) -> Uuid {
        self.id
    }

    pub fn bundle_hash(&self) -> B256 {
        // SAFETY: At this point, the bundle hash is guaranteed to be initialized.
        *self.bundle_hash.get().expect("bundle hash is not initialized")
    }
}

/// A simulated bundle.
#[derive(Debug, Clone)]
pub struct SimulatedBundle {
    /// The original validated bundle.
    validated: Arc<ValidatedBundle>,
    /// The simulated transactions in the bundle.
    simulated: Vec<SimulatedTx>,
    /// The total payment of the bundle.
    total_payment: Option<U256>,
    /// The result and state of the bundle, after simulating all transactions.
    result_and_state: Option<ResultAndStateMany>,
}

impl SimulatedBundle {
    /// Creates a new unsimulated bundle from a validated bundle.
    pub fn new(validated: Arc<ValidatedBundle>) -> Self {
        Self { validated, simulated: Vec::new(), total_payment: None, result_and_state: None }
    }

    /// Sets the simulation results for the bundle.
    pub fn set_simulation_results(
        &mut self,
        simulated: Vec<SimulatedTx>,
        total_payment: U256,
        result_and_state: ResultAndStateMany,
    ) {
        self.simulated = simulated;
        self.total_payment = Some(total_payment);
        self.result_and_state = Some(result_and_state);
    }

    /// Returns the nonce of the first transaction for the given sender in the bundle (if present).
    pub fn nonce_of(&self, sender: Address) -> Option<u64> {
        self.validated.transactions.iter().find(|tx| tx.sender() == sender).map(|tx| tx.nonce())
    }

    /// Returns true if the bundle has a transaction for the given sender.
    pub fn has_sender(&self, sender: Address) -> bool {
        self.validated.transactions.iter().any(|tx| tx.sender() == sender)
    }

    /// Returns true if the bundle has been simulated.
    pub fn is_simulated(&self) -> bool {
        self.result_and_state.is_some()
    }

    /// Returns the inner validated bundle.
    pub fn validated(&self) -> Arc<ValidatedBundle> {
        self.validated.clone()
    }

    /// Returns a reference to the inner validated bundle.
    pub fn validated_ref(&self) -> &ValidatedBundle {
        &self.validated
    }

    pub fn transactions(&self) -> &[SimulatedTx] {
        &self.simulated
    }

    pub fn transactions_mut(&mut self) -> &mut [SimulatedTx] {
        &mut self.simulated
    }

    pub fn into_transactions(self) -> Vec<SimulatedTx> {
        self.simulated
    }

    /// Returns the id of the bundle.
    pub fn id(&self) -> Uuid {
        self.validated.id()
    }

    /// Returns the weight of the bundle.
    pub fn weight(&self) -> U256 {
        // If we have the simulated payment, use that. Else sum the priority fees of the transactions.
        self.total_payment.unwrap_or_else(|| {
            self.validated_ref().transactions.iter().map(|tx| U256::from(tx.priority_fee_or_price())).sum()
        })
    }

    /// Returns the simulated payment of the bundle, if available.
    pub fn payment(&self) -> Option<U256> {
        self.total_payment
    }

    /// Returns the gas limit of the bundle (i.e. sum of all transactions' gas limits)
    pub fn gas_limit(&self) -> u64 {
        self.validated.transactions.iter().map(|tx| tx.gas_limit()).sum()
    }

    /// Returns the estimated DA size of the bundle (i.e. sum of all transactions' estimated DA sizes)
    pub fn estimated_da(&self) -> u64 {
        self.validated.transactions.iter().map(|tx| tx.estimated_tx_compressed_size()).sum()
    }

    /// Returns the gas used by the bundle if it has been simulated.
    pub fn gas_used(&self) -> u64 {
        self.simulated.iter().map(|tx| tx.gas_used()).sum()
    }

    /// Returns an iterator over the senders of the transactions in the bundle.
    pub fn senders(&self) -> impl Iterator<Item = Address> {
        self.validated.transactions.iter().map(|tx| tx.sender())
    }

    /// Returns the result and state of the bundle, if it has been simulated.
    pub fn result_and_state(&self) -> Option<&ResultAndStateMany> {
        self.result_and_state.as_ref()
    }

    /// Returns the simulation time of the bundle, if it has been simulated.
    pub fn sim_time(&self) -> Option<Nanos> {
        self.simulated.first().map(|tx| tx.sim_time)
    }
}
