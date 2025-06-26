use std::{collections::VecDeque, ops::Deref};

use alloy_consensus::TxEip1559;
use alloy_eips::eip2718::Encodable2718;
use bop_common::{
    signing::ECDSASigner,
    time::{Duration, Instant},
};
use op_alloy_consensus::OpTxEnvelope;
use op_alloy_rpc_types::OpTransactionReceipt;
use reqwest::{blocking::Client, Url};
use revm_primitives::{address, b256, Address, Bytes, TxKind, B256, U256};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use tracing::{info, level_filters::LevelFilter, warn};
use tracing_subscriber::EnvFilter;

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
    let max_gas_units = 21_000;
    let max_fee_per_gas = 1_000_000_000_000_000_000;
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
        nonce: response.result.unwrap().to(),
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

struct SpammerClient {
    client: Client,
    portal_url: Url,
    follower_url: Url,
}
impl Default for SpammerClient {
    fn default() -> Self {
        let client =
            reqwest::blocking::ClientBuilder::new().timeout(std::time::Duration::from_secs(10)).build().unwrap();
        let portal_url = Url::parse(&format!(
            "http://127.0.0.1:{}",
            std::env::var("PORTAL_PORT").expect("Please set PORTAL_PORT env var")
        ))
        .expect("invalid PORTAL_PORT");
        let follower_url = Url::parse(&format!(
            "http://127.0.0.1:{}",
            std::env::var("BOP_EL_PORT").expect("Please set BOP_EL_PORT env var")
        ))
        .expect("invalid BOP_EL_PORT");
        Self { client, portal_url, follower_url }
    }
}
impl SpammerClient {
    fn request<R: DeserializeOwned, S: AsRef<str>>(&self, method: S, params: Value) -> Option<R> {
        let payload = serde_json::json!({
            "id": 1,
            "jsonrpc": "2.0",
            "method": method.as_ref(),
            "params": params
        });

        let url = if method.as_ref() == "eth_sendRawTransaction" {
            self.portal_url.clone()
        } else {
            self.follower_url.clone()
        };

        let Ok(response) = self.client.post(url).json(&payload).send() else {
            tracing::warn!("couldn't send message to portal");
            return None;
        };
        let Ok(t) = response.text() else {
            warn!("couldn't get response text");
            return None;
        };
        let response: RpcResponse<R> = serde_json::from_str(&t).map_err(|e| warn!("couldn't parse {t}: {e}")).ok()?;
        response.result
    }

    fn get_nonce(&self, address: &Address) -> u64 {
        self.request::<U256, _>("eth_getTransactionCount", json!([address, "latest"])).unwrap_or_default().to()
    }

    fn get_balance(&self, address: &Address) -> U256 {
        self.request("eth_getBalance", json!([address])).unwrap_or_default()
    }

    fn get_receipt(&self, tx_id: B256) -> Option<OpTransactionReceipt> {
        self.request("eth_getTransactionReceipt", json!([tx_id]))
    }

    fn new_tx(&self, from: &mut TestAccount, to: Address, value: Option<U256>) -> (B256, U256) {
        let value = value.unwrap_or(U256::from_limbs([1, 0, 0, 0]));
        let tx = TxEip1559 {
            chain_id: 2151908,
            nonce: from.nonce,
            gas_limit: 21000,
            max_fee_per_gas: 10_000_000_000,
            max_priority_fee_per_gas: 1,
            to: TxKind::Call(to),
            value,
            ..Default::default()
        };

        from.balance -= value;
        from.nonce += 1;

        let signed_tx = from.sign_tx(tx).unwrap();
        let tx = OpTxEnvelope::Eip1559(signed_tx);
        let envelope = Bytes::from(tx.encoded_2718());
        self.request::<B256, _>("eth_sendRawTransaction", json!([envelope]));
        (tx.tx_hash(), value)
    }
}

#[derive(Clone, Debug)]
struct TestAccount {
    nonce: u64,
    balance: U256,
    signer: ECDSASigner,
}
impl TestAccount {
    pub fn random() -> Self {
        let signer = ECDSASigner::try_from_secret(B256::random().as_slice()).unwrap();
        Self { nonce: 0, balance: U256::ZERO, signer }
    }

    pub fn main(client: &SpammerClient) -> Self {
        let signer = ECDSASigner::try_from_secret(
            b256!("ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80").as_ref(),
        )
        .unwrap();
        let nonce = client.get_nonce(&signer.address);
        let balance = client.get_balance(&signer.address);
        Self { nonce, balance, signer }
    }
}
impl Deref for TestAccount {
    type Target = ECDSASigner;

    fn deref(&self) -> &Self::Target {
        &self.signer
    }
}

fn generate_random_accounts(n_accounts: usize) -> Vec<TestAccount> {
    (0..n_accounts).map(|_| TestAccount::random()).collect()
}
fn init_test_logging() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_max_level(LevelFilter::INFO)
        .with_thread_names(true)
        .with_file(true) // Enable file display
        .with_line_number(true) // Enable line number display
        .init(); // just console
}
#[ignore = "Requires setting PORTAL_RPC_URL"]
#[test]
fn tx_spammer() {
    init_test_logging();
    let client = SpammerClient::default();
    let mut main_account = TestAccount::main(&client);
    info!("using main account: {main_account:#?}");

    let mut test_accounts = generate_random_accounts(100);
    let mut pending = VecDeque::new();

    for t in &mut test_accounts {
        let (id, value) = client.new_tx(&mut main_account, t.address, Some(U256::from(1_000_000_000_000_000_000usize)));
        t.balance += value;
        pending.push_front((id, Instant::now()));
    }

    let mut tot = Duration::ZERO;
    let mut n = 0usize;
    while let Some((id, tstamp)) = pending.pop_front() {
        loop {
            if let Some(_) = client.get_receipt(id) {
                tot += tstamp.elapsed();
                n += 1;
                break;
            }
        }
    }
    println!("received {n} receipt after on avg {}", tot / n);

    loop {
        for i1 in 0..test_accounts.len() {
            for i2 in 0..test_accounts.len() {
                if i1 == i2 {
                    continue;
                }
                let to = test_accounts[i2].address;
                let a1 = &mut test_accounts[i1];
                let (id, value) = client.new_tx(a1, to, None);
                test_accounts[i2].balance += value;
                pending.push_front((id, Instant::now()));
            }
        }
        let mut tot = Duration::ZERO;
        let mut n = 0usize;
        while let Some((id, tstamp)) = pending.pop_front() {
            loop {
                if let Some(_receipt) = client.get_receipt(id) {
                    info!("got receipt for {id}");
                    tot += tstamp.elapsed();
                    n += 1;
                    break;
                }
            }
        }
        println!("received {n} receipt after on avg {}", tot / n);
    }
}

#[allow(dead_code)]
#[derive(Clone, Debug, serde::Deserialize)]
struct RpcResponse<T> {
    jsonrpc: String,
    id: u64,
    result: Option<T>,
}
