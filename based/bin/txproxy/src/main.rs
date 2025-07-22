use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use bop_common::utils::init_tracing;
use clap::Parser;
use cli::TxProxyArgs;
use server::TxProxyServer;
use tracing::info;
mod cli;
mod middleware;
mod server;

#[tokio::main]
async fn main() -> eyre::Result<()> {
    let args = TxProxyArgs::parse();
    let _guard = init_tracing((&args).into());

    let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), args.txproxy_port);
    let server = TxProxyServer::new(args.clone()).await?;

    info!(%addr, "starting TxProxy server");
    server.run(addr).await
}

// #[cfg(test)]
// mod tests {
//     use std::str::FromStr;

//     use alloy_network::{EthereumWallet, TransactionBuilder};
//     use alloy_primitives::{Address, U256};
//     use alloy_provider::{Provider, ProviderBuilder};
//     use alloy_rpc_types::TransactionRequest;
//     use alloy_signer_local::PrivateKeySigner;

//     #[tokio::test]
//     #[ignore]
//     async fn test_txproxy() -> eyre::Result<()> {
//         let from_wallet_private_key = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
//         let to_wallet_address = "0xF07208aB3090856D8444F2682a6ecd12832a2944";

//         let url = "http://0.0.0.0:8090".parse()?;
//         let provider = ProviderBuilder::new().on_http(url);

//         let url = "http://0.0.0.0:8080".parse()?;
//         let provider_full = ProviderBuilder::new().on_http(url);

//         let signer = PrivateKeySigner::from_str(from_wallet_private_key)?;
//         let from_wallet_address = signer.address();
//         let wallet = EthereumWallet::from(signer);

//         let to_wallet_address = Address::from_str(to_wallet_address)?;

//         let mut nonce = provider_full.get_transaction_count(from_wallet_address).await?;

//         loop {
//             let tx = TransactionRequest::default()
//                 .with_to(to_wallet_address)
//                 .with_nonce(nonce)
//                 .with_chain_id(63)
//                 .with_value(U256::from(100))
//                 .with_gas_limit(21_000)
//                 .with_max_priority_fee_per_gas(1_000_000_000)
//                 .with_max_fee_per_gas(20_000_000_000);

//             let tx_envelope = tx.build(&wallet).await?;
//             let _ = provider.send_tx_envelope(tx_envelope).await;

//             nonce += 1;

//             tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
//         }

//         Ok(())
//     }
// }
