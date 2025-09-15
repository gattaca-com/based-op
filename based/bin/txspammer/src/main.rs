use std::str::FromStr;
use std::time::Duration;
use std::time::Instant;

use alloy_primitives::U256;
use alloy_provider::Provider;
use alloy_provider::WsConnect;
use alloy_signer_local::PrivateKeySigner;
use bop_common::utils::init_tracing;
use clap::Parser;
use cli::TxSpammerArgs;
use reqwest::Url;
use tokio::sync::mpsc;
use tokio::time::interval;
use tokio::time::sleep;
use rand::Rng;
use rand::SeedableRng;

use crate::account::{Account, AccountGenerator, TxSpec};
mod account;
mod cli;

#[tokio::main]
async fn main() -> eyre::Result<()> {
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

    let sequencer = args.sequencer_url.clone().map(|url| {
        alloy_provider::ProviderBuilder::new()
            .disable_recommended_fillers()
            .on_http(url.parse().expect("invalid sequencer url"))
    });

    let chain_id = full_provider.get_chain_id().await?;

    let root_signer = PrivateKeySigner::from_str(&args.root_private_key).expect("invalid root private key");
    let mut root_account = Account::new(root_signer);
    root_account.refresh(full_provider.clone()).await.expect("failed to fetch root account nonce and balance");

    let mut account_generator = AccountGenerator::new(U256::from(123456789u64));

    let mut target_accounts =
        (0..args.num_accounts).map(|_| Account::new(account_generator.next())).collect::<Vec<Account>>();
    println!("Generated {} accounts", target_accounts.len());

    for account in target_accounts.iter_mut() {
        account.refresh(full_provider.clone()).await.expect("failed to fetch account nonce and balance");
    }

    let funding_amount = U256::from((args.funding_amount * 1e18) as u64);
    for account in target_accounts.iter_mut() {
        let amount_to_fund = funding_amount.saturating_sub(account.balance);
        if amount_to_fund.is_zero() {
            continue;
        }
        root_account
            .transfer(
                account,
                TxSpec {
                    chain_id,
                    gas_limit: args.gas_limit,
                    max_fee_per_gas: args.max_fee_per_gas,
                    max_priority_fee_per_gas: args.max_priority_fee_per_gas,
                    value: amount_to_fund,
                },
                full_provider.clone(),
                sequencer.clone(),
            )
            .await
            .expect("failed to fund account");
        println!("Funded account {:?} with {} ether", account.signer.address(), args.funding_amount);
        sleep(std::time::Duration::from_millis(50)).await;
    }
    sleep(std::time::Duration::from_secs(1)).await;
    for account in target_accounts.iter_mut() {
        account.refresh(full_provider.clone()).await.expect("failed to fetch account nonce and balance");
    }

    let tx_spec = TxSpec {
        chain_id,
        gas_limit: args.gas_limit,
        max_fee_per_gas: args.max_fee_per_gas,
        max_priority_fee_per_gas: args.max_priority_fee_per_gas,
        value: U256::from((args.tx_value * 1e18) as u64),
    };

    println!("Funding {} accounts with {} ether each", args.num_accounts, args.funding_amount);
    println!("Spamming transactions at {} tx/s", args.throughput);
    println!("Tx: {:?}", tx_spec);

    let accounts_clone = target_accounts.clone();
    let n = accounts_clone.len();
    let mut rng = rand::rngs::StdRng::seed_from_u64(1234567890);
    let mut counter = 0;

    let (tx, mut rx) = mpsc::channel::<(alloy_primitives::TxHash, Instant)>(1 << 16);

    let full_provider_clone = full_provider.clone();
    tokio::spawn(async move {
        while let Some((tx_hash, tx_out_time)) = rx.recv().await {
            let receipt = loop {
                match full_provider_clone.get_transaction_receipt(tx_hash).await {
                    Ok(Some(receipt)) => break receipt,
                    _ => {sleep(Duration::from_millis(5)).await;}
                }
            };
            let latency = tx_out_time.elapsed();
            println!("Tx {:?} confirmed in {} ms", tx_hash, latency.as_millis());
        }
    });

    for mut account in target_accounts.into_iter() {
        let args = args.clone();
        let tx_spec_cloned = tx_spec.clone();
        let full_provider = full_provider.clone();
        let sequencer = sequencer.clone();
        let mut accounts_clone = accounts_clone.clone();
        let mut rag2 = rand::rngs::StdRng::seed_from_u64(counter);
        let tx = tx.clone();
        counter += 1;
        sleep(Duration::from_millis(rng.random_range(0..100))).await;
        tokio::spawn(async move {
            let interval_nanos = 1_000_000_000f64 / args.throughput as f64 * args.num_accounts as f64;
            let interval_duration = Duration::from_nanos(interval_nanos as u64);
            let mut interval_send = interval(interval_duration);
            let mut refresh_timer = Instant::now();
            println!(
                "Account {:?} starts spamming every {:?}",
                account.signer.address(),
                interval_duration
            );
            loop {
                interval_send.tick().await;
                let to = &mut accounts_clone[rag2.random_range(0..n)];
                let tx_out_time = Instant::now();
                let tx_hash = account.transfer(to, tx_spec_cloned.clone(), full_provider.clone(), sequencer.clone()).await.expect("failed to send tx");
                tx.send((tx_hash, tx_out_time)).await.expect("failed to send tx hash to logger");

                if refresh_timer.elapsed() > Duration::from_secs(5) {
                    account.refresh_balance(full_provider.clone()).await.expect("failed to refresh account");
                    refresh_timer = Instant::now();
                }
            }
        });
    }

    tokio::signal::ctrl_c().await?;
    println!("Received Ctrl-C, shutting down");

    Ok(())
}
