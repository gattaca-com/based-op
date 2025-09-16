use std::{
    collections::HashMap,
    io::{Write, stdout},
    str::FromStr,
    time::{Duration, Instant},
};

use alloy_primitives::{
    U256,
    utils::{format_ether, parse_ether},
};
use alloy_provider::{Provider, WsConnect};
use alloy_signer_local::PrivateKeySigner;
use bop_common::{p2p::SignedVersionedMessage, utils::init_tracing};
use clap::Parser;
use cli::TxSpammerArgs;
use futures_util::stream::StreamExt;
use http::Uri;
use rand::{Rng, SeedableRng};
use reqwest::Url;
use tokio::{
    sync::mpsc,
    time::{interval, sleep},
};
use tokio_websockets::ClientBuilder;
use tracing::{info, warn};

use crate::account::{Account, AccountGenerator, TxSpec};
mod account;
mod cli;

#[tokio::main]
async fn main() -> eyre::Result<()> {
    // Initialize spammer
    let args = TxSpammerArgs::parse();
    let _guard = init_tracing((&args).into());

    let full_provider = match args.eth_rpc_url.starts_with("ws") {
        true => {
            let url: Url = args.eth_rpc_url.parse().expect("invalid eth rpc url");
            let ws_connect = WsConnect::new(url);
            alloy_provider::ProviderBuilder::new().disable_recommended_fillers().on_ws(ws_connect).await?
        }
        false => alloy_provider::ProviderBuilder::new()
            .disable_recommended_fillers()
            .on_http(args.eth_rpc_url.parse().expect("invalid eth rpc url")),
    };

    if args.sequencer_url.is_none() {
        warn!("");
        warn!(
            "Sequencer URL not specified, Submitting transactions via Eth RPC. This may lead to higher latency and lower throughput."
        );
        warn!("Consider using a sequencer for better performance. Use --sequencer.url to specify the URL.");
        warn!("eg: --sequencer.url http://127.0.0.1:9994");
        warn!("");
    }

    let sequencer = args.sequencer_url.clone().map(|url| {
        alloy_provider::ProviderBuilder::new()
            .disable_recommended_fillers()
            .on_http(url.parse().expect("invalid sequencer url"))
    });

    let chain_id = full_provider.get_chain_id().await?;
    info!("Eth RPC has chain id {}", chain_id);

    let root_signer = PrivateKeySigner::from_str(&args.root_private_key).expect("invalid root private key");
    let mut root_account = Account::new(root_signer);
    root_account.refresh(full_provider.clone()).await.expect("failed to fetch root account nonce and balance");
    info!("Root account {:?} has balance {} eth", root_account.signer.address(), format_ether(root_account.balance),);

    // Generate target accounts
    let mut account_generator = AccountGenerator::new(U256::from(123456789u64));
    let mut target_accounts =
        (0..args.num_accounts).map(|_| Account::new(account_generator.next())).collect::<Vec<Account>>();

    for account in target_accounts.iter_mut() {
        account.refresh(full_provider.clone()).await.expect("failed to fetch account nonce and balance");
        print!("Fetching {:?} balance: {} eth     \r", account.signer.address(), format_ether(account.balance));
        stdout().flush().unwrap();
    }

    // Fund target accounts
    let funding_amount = U256::from((args.funding_amount * 1e18) as u64);
    for account in target_accounts.iter_mut() {
        let amount_to_fund = funding_amount.saturating_sub(account.balance);
        if amount_to_fund.is_zero() || amount_to_fund < parse_ether("0.001").unwrap() {
            continue;
        }
        root_account
            .transfer(
                account,
                &TxSpec {
                    chain_id,
                    gas_limit: args.gas_limit,
                    max_fee_per_gas: args.max_fee_per_gas,
                    max_priority_fee_per_gas: args.max_priority_fee_per_gas,
                    value: amount_to_fund,
                },
                &full_provider,
                &sequencer,
            )
            .await
            .expect("failed to fund account");
        print!("Funding {:?} with {} eth     \r", account.signer.address(), format_ether(amount_to_fund));
        stdout().flush().unwrap();
        sleep(std::time::Duration::from_millis(10)).await;
    }
    sleep(std::time::Duration::from_secs(1)).await;

    // Verifying account states
    for account in target_accounts.iter_mut() {
        account.refresh(full_provider.clone()).await.expect("failed to fetch account nonce and balance");
        if funding_amount.saturating_sub(account.balance) > parse_ether("0.001").unwrap() {
            panic!(
                "Account {:?} has insufficient balance {} (Failed to transfer. Maybe max gas fee too low?)",
                account.signer.address(),
                account.balance
            );
        }
        print!("Verifying {:?} balance: {} eth     \r", account.signer.address(), format_ether(account.balance));
        stdout().flush().unwrap();
    }

    let tx_spec = TxSpec {
        chain_id,
        gas_limit: args.gas_limit,
        max_fee_per_gas: args.max_fee_per_gas,
        max_priority_fee_per_gas: args.max_priority_fee_per_gas,
        value: U256::from((args.tx_value * 1e18) as u64),
    };

    info!("Tx: {:?}", tx_spec);
    info!("Funded {} accounts with {} ether each", args.num_accounts, args.funding_amount);
    info!("Start spamming transactions at {} tx/s", args.throughput);

    // Set up receipt listener for latency measurement
    let accounts_clone = target_accounts.clone();
    let n = accounts_clone.len();
    let mut counter = 0;

    let (request_tx, mut request_rx) = mpsc::channel::<(alloy_primitives::TxHash, Instant)>(1 << 16);
    let (latency_tx, mut latency_rx) = mpsc::channel::<(alloy_primitives::TxHash, f64)>(1 << 16);

    let full_provider_clone = full_provider.clone();
    let args_clone = args.clone();
    tokio::spawn(async move {
        match args_clone.fragstream_url {
            Some(frag_url) => {
                let uri = Uri::from_str(&frag_url).expect("invalid frag stream url");
                let (mut client, _) =
                    ClientBuilder::from_uri(uri).connect().await.expect("failed to connect to frag stream");
                let mut requests = HashMap::new();
                while let Some(Ok(msg)) = client.next().await {
                    while let Ok((tx_hash, tx_out_time)) = request_rx.try_recv() {
                        requests.insert(tx_hash, tx_out_time);
                    }
                    let raw_text: Option<SignedVersionedMessage> =
                        msg.as_text().and_then(|s| serde_json::from_str::<SignedVersionedMessage>(s).ok());
                    match raw_text {
                        Some(msg) => {
                            let txs = msg.state_update.map(|su| su.receipts).unwrap_or_default();
                            for (tx_hash, _) in txs.iter() {
                                if let Some(tx_out_time) = requests.remove(tx_hash) {
                                    let latency = tx_out_time.elapsed();
                                    latency_tx
                                        .send((*tx_hash, latency.as_secs_f64()))
                                        .await
                                        .expect("failed to send latency");
                                }
                            }
                        }
                        None => {
                            warn!("Received non-text message: {:?}", msg);
                            continue;
                        }
                    }
                }
            }
            None => {
                warn!("");
                warn!(
                    "Frag stream URL not specified, falling back to RPC polling. Latency measurement may be inaccurate."
                );
                warn!(
                    "Consider using a frag stream for better latency measurement. Use --fragstream.url to specify the URL."
                );
                warn!("eg: --fragstream.url ws://0.0.0.0:9999/state_stream");
                warn!("");
                while let Some((tx_hash, tx_out_time)) = request_rx.recv().await {
                    let _ = loop {
                        match full_provider_clone.get_transaction_receipt(tx_hash).await {
                            Ok(Some(receipt)) => break receipt,
                            _ => {
                                sleep(Duration::from_millis(5)).await;
                            }
                        }
                    };
                    let latency = tx_out_time.elapsed();
                    latency_tx.send((tx_hash, latency.as_secs_f64())).await.expect("failed to send latency");
                }
            }
        }
    });

    // Start stats logger
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

            info!(
                "Last {}s: {} tx confirmed, TPS: {:.2}, Latency P50: {:.2}s, P95: {:.2}s, P99: {:.2}s",
                interval_secs,
                latencies.len(),
                tps,
                p50,
                p95,
                p99
            );
        }
    });

    // Start spamming
    for mut account in target_accounts.into_iter() {
        let args = args.clone();
        let tx_spec_cloned = tx_spec.clone();
        let full_provider = full_provider.clone();
        let sequencer = sequencer.clone();
        let mut accounts_clone = accounts_clone.clone();
        let mut rag2 = rand::rngs::StdRng::seed_from_u64(counter);
        let tx = request_tx.clone();
        counter += 1;
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
                let (tx_hash, _) =
                    account.transfer(to, &tx_spec_cloned, &full_provider, &sequencer).await.expect("failed to send tx");
                // let tx_out_time = Instant::now();
                tx.send((tx_hash, start_sending_at)).await.expect("failed to send tx hash to logger");

                if refresh_timer.elapsed() > Duration::from_secs(5) {
                    account.refresh_balance(full_provider.clone()).await.expect("failed to refresh account");
                    refresh_timer = Instant::now();
                }
            }
        });
    }

    tokio::signal::ctrl_c().await?;
    info!("Received Ctrl-C, shutting down");

    Ok(())
}

struct Percentile {
    pub data: Vec<f64>,
    pub sorted: bool,
}

impl Percentile {
    fn new() -> Self {
        Self { data: Vec::new(), sorted: false }
    }

    fn add(&mut self, value: f64) {
        self.data.push(value);
        self.sorted = false;
    }

    fn len(&self) -> usize {
        self.data.len()
    }

    fn percentile(&mut self, p: f64) -> Option<f64> {
        if self.data.is_empty() {
            return None;
        }
        if !self.sorted {
            self.data.sort_by(|a, b| a.partial_cmp(b).unwrap());
            self.sorted = true;
        }
        let rank = (p / 100.0) * (self.data.len() - 1) as f64;
        let lower_index = rank.floor() as usize;
        let upper_index = rank.ceil() as usize;
        if lower_index == upper_index {
            Some(self.data[lower_index])
        } else {
            let lower_value = self.data[lower_index];
            let upper_value = self.data[upper_index];
            Some(lower_value + (upper_value - lower_value) * (rank - lower_index as f64))
        }
    }
}
