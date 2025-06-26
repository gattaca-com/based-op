use std::fmt;

use alloy_consensus::{SignableTransaction, Signed, TxEip1559};
use alloy_eips::Encodable2718;
use alloy_network::TxSignerSync;
use alloy_primitives::{hex, hex::FromHexError, Address, Signature, B256, U256};
use alloy_rlp::Bytes;
use alloy_signer::SignerSync;
use alloy_signer_local::PrivateKeySigner;
use http::{Request, Uri};
use mio_httpc::CallRef;
use op_alloy_consensus::OpTxEnvelope;
use rand::RngCore;
use revm_primitives::TxKind;

use crate::communication::{walkie_talkie::RpcResponse, WalkieTalkie};

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
    pub fn new(secret: B256) -> Result<Self, SignerError> {
        let secret = PrivateKeySigner::from_bytes(&secret).map_err(alloy_signer::Error::Ecdsa)?;
        let address = secret.address();

        Ok(Self { address, secret })
    }

    pub fn try_from_secret(secret: &[u8]) -> Result<Self, SignerError> {
        let secret: B256 =
            secret.try_into().map_err(|_| SignerError::SignerError("Failed to convert secret to B256".to_string()))?;
        Self::new(secret)
    }

    pub fn try_from_hex(hex: &str) -> Result<Self, SignerError> {
        let bytes = hex::decode(hex)?;
        Self::try_from_secret(&bytes)
    }

    #[inline]
    pub fn sign_message(&self, message: B256) -> Result<Signature, SignerError> {
        let sig = self.secret.sign_hash_sync(&message)?;
        Ok(sig)
    }

    /// Creates a new signer with randomly generated private key
    pub fn random() -> Self {
        let mut bytes = [0u8; 32];
        rand::rng().fill_bytes(&mut bytes);
        Self::try_from_secret(&bytes).expect("valid random 32 bytes should create valid secret key")
    }

    pub fn sign_tx<T: SignableTransaction<Signature>>(&self, mut tx: T) -> Result<Signed<T>, SignerError> {
        let signature = self.secret.sign_transaction_sync(&mut tx)?;
        let signed = tx.into_signed(signature);
        Ok(signed)
    }

    pub fn nonce(&self, walkie_talkie: &mut WalkieTalkie, uri: Uri) -> Option<u64> {
        let Ok(payload) = serde_json::to_vec(&serde_json::json!({
            "id": 1,
            "jsonrpc": "2.0",
            "method": "eth_getTransactionCount",
            "params": [self.address.to_string(), "latest"]
        })) else {
            tracing::warn!("couldn't create nonce request payload");
            return None;
        };

        let Ok(req) =
            Request::builder().header("content-type", "application/json").method("POST").uri(uri.clone()).body(payload)
        else {
            tracing::warn!("couldn't create request");
            return None;
        };

        let Ok((resp, body)) = walkie_talkie.send_blocking(req) else {
            tracing::warn!("issue sending nonce request to portal");
            return None;
        };

        if resp.status != 200 {
            tracing::warn!(
                "got response status {}: {resp:?} - {}",
                resp.status,
                String::from_utf8(body).unwrap_or_default()
            );
            return None;
        }

        let Ok(nonce) = serde_json::from_slice::<RpcResponse<U256>>(&body) else {
            tracing::warn!("couldn't parse nonce response from {}", String::from_utf8(body).unwrap_or_default());
            return None;
        };
        nonce.result.map(|t| t.to())
    }

    pub fn send_transfer(
        &self,
        walkie_talkie: &mut WalkieTalkie,
        uri: Uri,
        chain_id: u64,
        to_address: Address,
        value: U256,
        nonce: Option<u64>,
    ) -> Option<CallRef> {
        let nonce = nonce.or_else(|| self.nonce(walkie_talkie, uri.clone()))?;
        let max_gas_units = 21_000;
        let max_fee_per_gas = 1_000_000_000;
        let max_priority_fee_per_gas = 0;

        let tx = TxEip1559 {
            chain_id,
            nonce,
            gas_limit: max_gas_units,
            max_fee_per_gas,
            max_priority_fee_per_gas,
            to: TxKind::Call(to_address),
            value,
            ..Default::default()
        };
        let signed_tx = self.sign_tx(tx).unwrap();
        let tx = OpTxEnvelope::Eip1559(signed_tx);
        let envelope = Bytes::from(tx.encoded_2718());

        let Ok(payload) = serde_json::to_vec(&serde_json::json!({
            "id": 1,
            "jsonrpc": "2.0",
            "method": "eth_sendRawTransaction",
            "params": [envelope]
        })) else {
            tracing::warn!("couldn't create tx payload");
            return None;
        };

        let Ok(req) =
            Request::builder().method("POST").uri(uri.clone()).header("content-type", "application/json").body(payload)
        else {
            tracing::warn!("couldn't create request");
            return None;
        };

        walkie_talkie
            .send(req)
            .inspect_err(|e| {
                tracing::warn!("issue sending nonce request to portal: {e}");
            })
            .ok()
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
