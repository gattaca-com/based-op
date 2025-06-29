use std::time::Duration;

use alloy_consensus::TxEip1559;
use alloy_eips::{Decodable2718, eip2718::Encodable2718};
use alloy_primitives::{Address, TxKind, U256, b256, hex};
use bop_common::{
    fabric::{CommitmentRequest, FRAG_COMMITMENT_TYPE, FabricGatewayApiClient},
    p2p::{SignedVersionedMessage, VersionedMessage},
    signing::ECDSASigner,
};
use clap::Parser;
use jsonrpsee::http_client::HttpClientBuilder;
use op_alloy_consensus::OpTxEnvelope;
use reqwest::Url;
use serde::Deserialize;
use serde_json::json;
use tracing::{error, info};

#[derive(Parser, Debug)]
#[command(version, about = "Test fabric commitment RPC")]
struct Args {
    #[arg(long, default_value = "http://0.0.0.0:9998")]
    gateway_url: Url,

    /// ETH RPC URL for getting nonce
    #[arg(long, default_value = "http://0.0.0.0:8545")]
    eth_rpc_url: Url,

    /// Private key for signing (hex string, defaults to test account)
    #[arg(long)]
    private_key: Option<String>,
}

#[allow(dead_code)]
#[derive(Clone, Debug, Deserialize)]
struct RpcResponse<T> {
    jsonrpc: String,
    id: u64,
    result: Option<T>,
}

async fn get_nonce(eth_client: &reqwest::Client, eth_url: &Url, address: Address) -> eyre::Result<u64> {
    let payload = json!({
        "id": 1,
        "jsonrpc": "2.0",
        "method": "eth_getTransactionCount",
        "params": [address.to_string(), "latest"]
    });

    let response = eth_client.post(eth_url.clone()).json(&payload).send().await?;
    let response_text = response.text().await?;
    let rpc_response: RpcResponse<U256> = serde_json::from_str(&response_text)?;

    Ok(rpc_response.result.unwrap_or_default().to())
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env()).init();

    let args = Args::parse();
    info!(?args, "Starting fabric commitment test");

    let commitment_client =
        HttpClientBuilder::default().request_timeout(Duration::from_secs(30)).build(args.gateway_url)?;

    let eth_client = reqwest::Client::builder().timeout(Duration::from_secs(10)).build()?;

    let signer = if let Some(pk) = args.private_key {
        let key_bytes = hex::decode(pk.strip_prefix("0x").unwrap_or(&pk))?;
        ECDSASigner::try_from_secret(&key_bytes)?
    } else {
        // Default test account
        ECDSASigner::try_from_secret(
            b256!("ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80").as_ref(),
        )?
    };

    // Create test transaction
    let nonce = get_nonce(&eth_client, &args.eth_rpc_url, signer.address).await?;
    let tx = TxEip1559 {
        chain_id: 43444,
        nonce,
        gas_limit: 21000,
        max_fee_per_gas: 10_000_000_000,
        max_priority_fee_per_gas: 1_000_000_000,
        to: TxKind::Call(signer.address),
        value: U256::from(1000000000000000u64),
        ..Default::default()
    };

    let signed_tx = signer.sign_tx(tx)?;
    let op_tx = OpTxEnvelope::Eip1559(signed_tx);
    let encoded_tx = op_tx.encoded_2718();
    info!(?op_tx, sender = %signer.address, hash = %op_tx.tx_hash(), "Created test transaction");

    // Create commitment request
    let commitment_request = CommitmentRequest {
        commitment_type: FRAG_COMMITMENT_TYPE,
        payload: encoded_tx.into(),
        slasher: Address::random(),
    };
    info!(?commitment_request, "Sending commitment request");

    match commitment_client.post_commitment(commitment_request.clone()).await {
        Ok(signed_commitment) => {
            info!(?signed_commitment, "Received signed commitment");

            // Deserialise the response into a SignedFrag and verify that the tx is in the frag
            let msg: SignedVersionedMessage = serde_json::from_slice(&signed_commitment.commitment.payload)?;
            let VersionedMessage::FragV0(frag) = &msg.message else {
                return Err(eyre::eyre!("Expected FragV0 message got {:?}", msg.message));
            };
            if frag
                .txs
                .iter()
                .filter_map(|t| OpTxEnvelope::decode_2718(&mut t.as_ref()).ok())
                .any(|t| *t.hash() == op_tx.tx_hash())
            {
                info!("SUCCESS: Transaction found in frag! Commitment received");
            } else {
                return Err(eyre::eyre!("Transaction not found in frag commitment"));
            }
        }
        Err(e) => {
            error!("FAILED: Could not get commitment: {}", e);
            return Err(e.into());
        }
    }

    Ok(())
}
