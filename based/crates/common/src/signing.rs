use std::fmt;

use alloy_consensus::{SignableTransaction, Signed};
use alloy_network::TxSignerSync;
use alloy_primitives::{hex, hex::FromHexError, Address, PrimitiveSignature, B256};
use alloy_signer::SignerSync;
use alloy_signer_local::PrivateKeySigner;
use op_alloy_consensus::OpTxEnvelope;
use rand::RngCore;

#[derive(Debug, thiserror::Error)]
pub enum SignerError {
    #[error("From hex error: {0}")]
    FromHexError(#[from] FromHexError),
    #[error("{0}")]
    AlloySignerError(#[from] alloy_signer::Error),
    #[error("Signer error: {0}")]
    SignerError(String),
}

#[derive(Clone)]
pub struct ECDSASigner {
    pub address: Address,
    pub secret: PrivateKeySigner,
}

impl ECDSASigner {
    pub fn try_from_secret(secret: &[u8]) -> Result<Self, SignerError> {
        let secret: B256 =
            secret.try_into().map_err(|_| SignerError::SignerError("Failed to convert secret to B256".to_string()))?;
        let secret = PrivateKeySigner::from_bytes(&secret).map_err(alloy_signer::Error::Ecdsa)?;
        let address = secret.address();

        Ok(Self { address, secret })
    }

    pub fn try_from_hex(hex: &str) -> Result<Self, SignerError> {
        let bytes = hex::decode(hex)?;
        Self::try_from_secret(&bytes)
    }

    #[inline]
    pub fn sign_message(&self, message: B256) -> Result<PrimitiveSignature, SignerError> {
        let sig = self.secret.sign_hash_sync(&message)?;
        Ok(sig)
    }

    /// Creates a new signer with randomly generated private key
    pub fn random() -> Self {
        let mut bytes = [0u8; 32];
        rand::rng().fill_bytes(&mut bytes);
        Self::try_from_secret(&bytes).expect("valid random 32 bytes should create valid secret key")
    }

    pub fn sign_tx<T: SignableTransaction<PrimitiveSignature>>(&self, mut tx: T) -> Result<Signed<T>, SignerError> {
        let signature = self.secret.sign_transaction_sync(&mut tx)?;
        let signed = tx.into_signed(signature);
        Ok(signed)
    }
}

impl fmt::Debug for ECDSASigner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ECDSASigner").field("address", &self.address).finish()
    }
}

impl Default for ECDSASigner {
    fn default() -> Self {
        Self::random()
    }
}
