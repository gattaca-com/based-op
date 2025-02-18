use alloy_consensus::TxEip1559;
use alloy_eips::eip2718::Encodable2718;
use bop_common::{
    signing::ECDSASigner,
    time::{Duration, Instant},
};
use op_alloy_consensus::OpTxEnvelope;
use op_alloy_rpc_types::OpTransactionReceipt;
use reqwest::Url;
use revm_primitives::{address, b256, Bytes, TxKind, B256, U256};

#[ignore = "Requires setting PORTAL_RPC_URL"]
#[test]
fn tx_roundtrip() {
    let portal_url = Url::parse(&std::env::var("PORTAL_RPC_URL").expect("Please set PORTAL_RPC_URL env var"))
        .expect("invalid PORTAL_RPC_URL");

    let from_account = address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266");
    let signing_wallet = ECDSASigner::try_from_secret(
        b256!("ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80").as_ref(),
    )
    .unwrap();

    let client = reqwest::blocking::ClientBuilder::new().timeout(std::time::Duration::from_secs(10)).build().unwrap();

    let value = U256::from_limbs([1, 0, 0, 0]);
    let chain_id = 2151908;
    let to_account = address!("0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266");
    let max_gas_units = 21000;
    let max_fee_per_gas = 1_258_615_255_000;
    let max_priority_fee_per_gas = 1_000;

    let payload = serde_json::json!({
        "id": 1,
        "jsonrpc": "2.0",
        "method": "eth_getTransactionCount",
        "params": [from_account.to_string(), "latest"]
    });

    let response = client.post(portal_url.clone()).json(&payload).send().expect("couldn't send message to portal");
    let t = response.text().expect("couldn't get response text");
    println!("response {t}");
    let response: RpcResponse<U256> = serde_json::from_str(&t).expect("couldn't parse tx response");

    let tx = TxEip1559 {
        chain_id,
        nonce: response.result.to(),
        gas_limit: max_gas_units,
        max_fee_per_gas,
        max_priority_fee_per_gas,
        to: TxKind::Call(to_account),
        value,
        ..Default::default()
    };
    let signed_tx = signing_wallet.sign_tx(tx).unwrap();
    let tx = OpTxEnvelope::Eip1559(signed_tx);
    let envelope = Bytes::from(tx.encoded_2718());

    let payload = serde_json::json!({
        "id": 1,
        "jsonrpc": "2.0",
        "method": "eth_sendRawTransaction",
        "params": [envelope]
    });

    let response = client.post(portal_url.clone()).json(&payload).send().expect("couldn't send message to portal");

    let response: RpcResponse<B256> = serde_json::from_str(&response.text().expect("couldn't get response text"))
        .expect("couldn't parse tx response");
    let curt = Instant::now();

    let payload = serde_json::json!({
        "id": 1,
        "jsonrpc": "2.0",
        "method": "eth_getTransactionReceipt",
        "params": [response.result]
    });
    loop {
        let response = client.post(portal_url.clone()).json(&payload).send().expect("couldn't send message to portal");
        let response: RpcResponse<Option<OpTransactionReceipt>> =
            serde_json::from_str(&response.text().expect("couldn't get response text"))
                .expect("couldn't parse tx response");
        if let Some(response) = response.result {
            println!("{:#?}", response);
            println!("after {}", curt.elapsed());
            break;
        }
        // bop_common::time::Duration::from_millis(20).sleep();
    }
}

#[ignore = "Requires setting PORTAL_RPC_URL"]
#[test]
fn tx_spammer() {
    let portal_url = Url::parse(&std::env::var("PORTAL_RPC_URL").expect("Please set PORTAL_RPC_URL env var"))
        .expect("invalid PORTAL_RPC_URL");

    let from_account = address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266");
    let signing_wallet = ECDSASigner::try_from_secret(
        b256!("ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80").as_ref(),
    )
    .unwrap();

    let client = reqwest::blocking::ClientBuilder::new().timeout(std::time::Duration::from_secs(10)).build().unwrap();

    let value = U256::from_limbs([1, 0, 0, 0]);
    let chain_id = 2151908;
    let to_account = address!("0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266");
    let max_gas_units = 21000;
    let max_fee_per_gas = 1_258_615_255_000;
    let max_priority_fee_per_gas = 1_000;

    let payload = serde_json::json!({
        "id": 1,
        "jsonrpc": "2.0",
        "method": "eth_getTransactionCount",
        "params": [from_account.to_string(), "latest"]
    });

    let response = client.post(portal_url.clone()).json(&payload).send().expect("couldn't send message to portal");
    let t = response.text().expect("couldn't get response text");
    println!("response {t}");
    let response: RpcResponse<U256> = serde_json::from_str(&t).expect("couldn't parse tx response");

    let mut nonce = response.result.to();
    let n = 10000usize;

    loop {
        let mut timestamps = vec![];
        let mut tx_ids = vec![];
        let mut tot = Duration::ZERO;

        for _ in 0..n {
            let tx = TxEip1559 {
                chain_id,
                nonce,
                gas_limit: max_gas_units,
                max_fee_per_gas,
                max_priority_fee_per_gas,
                to: TxKind::Call(to_account),
                value,
                ..Default::default()
            };
            let signed_tx = signing_wallet.sign_tx(tx).unwrap();
            let tx = OpTxEnvelope::Eip1559(signed_tx);
            let envelope = Bytes::from(tx.encoded_2718());

            let payload = serde_json::json!({
                "id": 1,
                "jsonrpc": "2.0",
                "method": "eth_sendRawTransaction",
                "params": [envelope]
            });

            timestamps.push(Instant::now());
            let response =
                client.post(portal_url.clone()).json(&payload).send().expect("couldn't send message to portal");
            let t = response.text().expect("couldn't get response text");
            let response: RpcResponse<B256> = serde_json::from_str(&t).expect("couldn't parse tx response");
            tx_ids.push(Some(response.result));
            nonce += 1;
        }
        let mut did_one = true;
        while did_one {
            did_one = false;
            for (timestamp, tx_id) in timestamps.iter().zip(tx_ids.iter_mut()) {
                if tx_id.is_none() {
                    continue;
                }
                did_one = true;
                let payload = serde_json::json!({
                    "id": 1,
                    "jsonrpc": "2.0",
                    "method": "eth_getTransactionReceipt",
                    "params": [tx_id.as_ref().unwrap()]
                });
                let response =
                    client.post(portal_url.clone()).json(&payload).send().expect("couldn't send message to portal");
                let response: RpcResponse<Option<OpTransactionReceipt>> =
                    serde_json::from_str(&response.text().expect("couldn't get response text"))
                        .expect("couldn't parse tx response");
                if let Some(_) = response.result {
                    let dur = timestamp.elapsed();
                    tot += dur;
                    *tx_id = None;
                }
            }
        }
        println!("avg receipt response time for {n} reqs: {}", tot/n);
    }
}

#[allow(dead_code)]
#[derive(Clone, Debug, serde::Deserialize)]
struct RpcResponse<T> {
    jsonrpc: String,
    id: u64,
    result: T,
}
