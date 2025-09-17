use std::{
    collections::HashMap,
    io::{Write, stdout},
    str::FromStr,
    time::{Duration, Instant},
};

use alloy_primitives::{
    TxHash, U256,
    utils::{format_ether, parse_ether},
};
use alloy_provider::{Provider, RootProvider, WsConnect};
use alloy_signer_local::PrivateKeySigner;
use bop_common::{p2p::SignedVersionedMessage, utils::init_tracing};
use clap::Parser;
use cli::TxSpammerArgs;
use futures_util::stream::StreamExt;
use http::Uri;
use rand::{Rng, SeedableRng};
use reqwest::Url;
use tokio::{
    sync::mpsc::{self, Receiver, Sender},
    time::{interval, sleep},
};
use tokio_websockets::ClientBuilder;
use tracing::{info, warn};

use crate::{
    account::{Account, AccountGenerator, TxSpec},
    utils::Percentile,
};

mod account;
mod cli;
mod utils;

struct TxSpammer {
    full_provider: RootProvider,
    sequencer: Option<RootProvider>,
    root_account: Account,
    target_accounts: Vec<Account>,
    tx_spec: TxSpec,
    args: TxSpammerArgs,
    request_rx: Option<Receiver<(TxHash, Instant)>>,
    request_tx: Sender<(TxHash, Instant)>,
    latency_rx: Option<Receiver<(TxHash, f64)>>,
    latency_tx: Sender<(TxHash, f64)>,
}

impl TxSpammer {
    pub async fn new(args: TxSpammerArgs) -> Self {
        // Initialize spammer
        let full_provider = match args.eth_rpc_url.starts_with("ws") {
            true => {
                let url: Url = args.eth_rpc_url.parse().expect("invalid eth rpc url");
                let ws_connect = WsConnect::new(url);
                alloy_provider::ProviderBuilder::new()
                    .disable_recommended_fillers()
                    .on_ws(ws_connect)
                    .await
                    .expect("failed to connect to eth rpc")
            }
            false => alloy_provider::ProviderBuilder::new()
                .disable_recommended_fillers()
                .on_http(args.eth_rpc_url.parse().expect("invalid eth rpc url")),
        };

        let sequencer = args.sequencer_url.clone().map(|url| {
            alloy_provider::ProviderBuilder::new()
                .disable_recommended_fillers()
                .on_http(url.parse().expect("invalid sequencer url"))
        });

        let root_signer = PrivateKeySigner::from_str(&args.root_private_key).expect("invalid root private key");
        let root_account = Account::new(root_signer);

        let tx_spec = TxSpec {
            chain_id: full_provider.get_chain_id().await.expect("failed to get chain id"),
            gas_limit: args.gas_limit,
            max_fee_per_gas: args.max_fee_per_gas,
            max_priority_fee_per_gas: args.max_priority_fee_per_gas,
            value: U256::from((args.tx_value * 1e18) as u64),
        };

        let (request_tx, request_rx) = mpsc::channel::<(TxHash, Instant)>(1 << 16);
        let (latency_tx, latency_rx) = mpsc::channel::<(TxHash, f64)>(1 << 16);

        Self {
            full_provider,
            sequencer,
            root_account,
            target_accounts: Vec::new(),
            tx_spec,
            args,
            request_rx: Some(request_rx),
            request_tx,
            latency_rx: Some(latency_rx),
            latency_tx,
        }
    }

    pub async fn prepare_accounts(&mut self) {
        let provider = &self.full_provider;

        // Getting root account nonce and balance
        self.root_account.refresh(provider).await.expect("failed to fetch root account nonce and balance");
        info!(
            "Root account {:?} has balance {} eth",
            self.root_account.signer.address(),
            format_ether(self.root_account.balance),
        );

        // Generate target accounts
        let num_accounts = self.args.num_accounts;
        let mut account_generator = AccountGenerator::new(U256::from(123456789u64));
        let mut target_accounts =
            (0..num_accounts).map(|_| Account::new(account_generator.next())).collect::<Vec<Account>>();

        // Fetching target accounts nonce and balance
        for account in target_accounts.iter_mut() {
            account.refresh(provider).await.expect("failed to fetch account nonce and balance");
            print!("Fetching {:?} balance: {} eth     \r", account.signer.address(), format_ether(account.balance));
            stdout().flush().unwrap();
        }

        self.target_accounts = target_accounts;
    }

    pub async fn fund_target_accounts(&mut self) {
        let provider = &self.full_provider;
        let sequencer = &self.sequencer;
        let funding_amount = U256::from((self.args.funding_amount * 1e18) as u64);
        let threshold = parse_ether("0.01").unwrap();

        // Fund target accounts
        for account in self.target_accounts.iter_mut() {
            let amount_to_fund = funding_amount.saturating_sub(account.balance);
            if amount_to_fund.is_zero() || amount_to_fund < threshold {
                continue;
            }
            self.root_account
                .transfer(
                    account,
                    &TxSpec {
                        chain_id: self.tx_spec.chain_id,
                        gas_limit: self.tx_spec.gas_limit,
                        max_fee_per_gas: self.tx_spec.max_fee_per_gas,
                        max_priority_fee_per_gas: self.tx_spec.max_priority_fee_per_gas,
                        value: amount_to_fund,
                    },
                    provider,
                    sequencer,
                )
                .await
                .expect("failed to fund account");
            print!("Funding {:?} with {} eth     \r", account.signer.address(), format_ether(amount_to_fund));
            stdout().flush().unwrap();
            sleep(std::time::Duration::from_millis(10)).await;
        }
        sleep(std::time::Duration::from_secs(1)).await;

        // Verifying account states
        for account in self.target_accounts.iter_mut() {
            account.refresh(provider).await.expect("failed to fetch account nonce and balance");
            if funding_amount.saturating_sub(account.balance) > threshold {
                panic!(
                    "Account {:?} has insufficient balance {} (Failed to transfer. Maybe max gas fee too low?)",
                    account.signer.address(),
                    account.balance
                );
            }
            print!("Verifying {:?} balance: {} eth     \r", account.signer.address(), format_ether(account.balance));
            stdout().flush().unwrap();
        }
    }

    pub fn spawn_receipt_listener_frag_stream(&mut self) {
        let mut request_rx = self.request_rx.take().expect("request receiver already taken");
        let latency_tx = self.latency_tx.clone();
        let frag_url = self.args.fragstream_url.clone().expect("frag stream url not specified");
        tokio::spawn(async move {
            let uri = Uri::from_str(&frag_url).expect("invalid frag stream url");
            let (mut client, _) =
                ClientBuilder::from_uri(uri).connect().await.expect("failed to connect to frag stream");
            let mut requests = HashMap::new();
            while let Some(Ok(msg)) = client.next().await {
                while let Ok((tx_hash, tx_out_time)) = request_rx.try_recv() {
                    requests.insert(tx_hash, tx_out_time);
                }
                let stream_data = msg.as_text().and_then(|s| serde_json::from_str::<SignedVersionedMessage>(s).ok());
                if let Some(msg) = stream_data {
                    let txs = msg.state_update.map(|su| su.receipts).unwrap_or_default();
                    for (tx_hash, _) in txs.iter() {
                        if let Some(tx_out_time) = requests.remove(tx_hash) {
                            let latency = tx_out_time.elapsed();
                            latency_tx.send((*tx_hash, latency.as_secs_f64())).await.expect("failed to send latency");
                        }
                    }
                }
            }
        });
    }

    pub fn spawn_receipt_listener_rpc_polling(&mut self) {
        let mut request_rx = self.request_rx.take().expect("request receiver already taken");
        let latency_tx = self.latency_tx.clone();
        let provider = self.full_provider.clone();
        tokio::spawn(async move {
            let mut requests = HashMap::new();
            while let Some((tx_hash, tx_out_time)) = request_rx.recv().await {
                requests.insert(tx_hash, tx_out_time);
                let _ = loop {
                    match provider.get_transaction_receipt(tx_hash).await {
                        Ok(Some(receipt)) => break receipt,
                        _ => {
                            sleep(Duration::from_millis(5)).await;
                        }
                    }
                };
                let latency = requests.remove(&tx_hash).unwrap().elapsed();
                latency_tx.send((tx_hash, latency.as_secs_f64())).await.expect("failed to send latency");
            }
        });
    }

    pub fn spawn_stats_logger(&mut self) {
        // Start stats logger
        let mut latency_rx = self.latency_rx.take().expect("latency receiver already taken");
        tokio::spawn(async move {
            let interval_secs = 5;
            let mut info_interval = interval(Duration::from_secs(interval_secs));
            loop {
                info_interval.tick().await;

                let mut latencies = Percentile::new();
                while let Ok((_, latency)) = latency_rx.try_recv() {
                    latencies.add(latency);
                }

                let tps = latencies.len() as f64 / interval_secs as f64;

                let p50 = latencies.percentile(50.0).unwrap_or(0.0);
                let p95 = latencies.percentile(95.0).unwrap_or(0.0);
                let p99 = latencies.percentile(99.0).unwrap_or(0.0);

                let fast_txs = latencies.data.iter().filter(|&&latency| latency < 1.0).count();
                let fast_txs_percent = if latencies.len() > 0 {
                    fast_txs as f64 / latencies.len() as f64 * 100.0
                } else {
                    0.0
                };

                info!(
                    "Last {}s: {} tx confirmed, TPS: {:.2}, Latency P50: {:.2}s, P95: {:.2}s, P99: {:.2}s, txs<1s: {:.2}%",
                    interval_secs,
                    latencies.len(),
                    tps,
                    p50,
                    p95,
                    p99,
                    fast_txs_percent
                );
            }
        });
    }

    pub fn spawn_spammer(&self) {
        for (counter, mut account) in self.target_accounts.clone().into_iter().enumerate() {
            let request_tx = self.request_tx.clone();
            let args = self.args.clone();
            let tx_spec = self.tx_spec.clone();
            let full_provider = self.full_provider.clone();
            let sequencer_provider = self.sequencer.clone();
            let mut accounts_clone = self.target_accounts.clone();
            let n = accounts_clone.len();
            let mut rag2 = rand::rngs::StdRng::seed_from_u64(counter as u64);
            tokio::spawn(async move {
                let interval_nanos = 1_000_000_000f64 / args.throughput as f64 * args.num_accounts as f64;
                sleep(Duration::from_nanos(rag2.random_range(0..(interval_nanos as u64)))).await;
                let interval_duration = Duration::from_nanos(interval_nanos as u64);
                let mut interval_send = interval(interval_duration);
                let mut refresh_timer = Instant::now();
                loop {
                    interval_send.tick().await;
                    let to = &mut accounts_clone[rag2.random_range(0..n)];
                    let start_sending_at = Instant::now();
                    let tx_hash = account
                        .transfer(to, &tx_spec, &full_provider, &sequencer_provider)
                        .await
                        .expect("failed to send tx");
                    request_tx.send((tx_hash, start_sending_at)).await.expect("failed to send tx hash to logger");

                    if refresh_timer.elapsed() > Duration::from_secs(5) {
                        account.refresh_balance(&full_provider).await.expect("failed to refresh account");
                        refresh_timer = Instant::now();
                    }
                }
            });
        }
    }
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
    // Initialize spammer
    let args = TxSpammerArgs::parse();
    let _guard = init_tracing((&args).into());

    if args.sequencer_url.is_none() {
        warn!("");
        warn!("Sequencer URL not specified, Submitting transactions via Eth RPC.");
        warn!("This may lead to higher latency and lower throughput.");
        warn!("Consider using a sequencer for better performance. Use --sequencer.url to specify the URL.");
        warn!("eg: --sequencer.url http://127.0.0.1:9994");
        warn!("");
    }

    let mut spammer = TxSpammer::new(args).await;
    spammer.prepare_accounts().await;
    spammer.fund_target_accounts().await;

    if spammer.args.fragstream_url.is_some() {
        spammer.spawn_receipt_listener_frag_stream();
    } else {
        warn!("");
        warn!("Frag stream URL not specified, falling back to RPC polling. Latency measurement may be inaccurate.");
        warn!("Consider using a frag stream for better latency measurement. Use --fragstream.url to specify the URL.");
        warn!("eg: --fragstream.url ws://0.0.0.0:9999/state_stream");
        warn!("");
        spammer.spawn_receipt_listener_rpc_polling();
    }

    spammer.spawn_stats_logger();
    spammer.spawn_spammer();

    tokio::signal::ctrl_c().await.expect("failed to listen for ctrl-c");
    info!("Received Ctrl-C, shutting down...");

    Ok(())
}
