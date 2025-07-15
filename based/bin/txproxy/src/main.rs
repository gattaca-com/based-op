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

//
#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use alloy::{
        network::{EthereumWallet, TransactionBuilder, eip2718::Encodable2718},
        primitives::U256,
        providers::{Provider, ProviderBuilder},
        rpc::types::TransactionRequest,
        signers::local::PrivateKeySigner,
    };
    use alloy_primitives::Address;
    use eyre::Result;

    #[tokio::test]
    #[ignore = "This test is for manual testing of sending raw transactions"]
    async fn test_send_raw_transaction() -> Result<()> {
        let rpc_url = "http://localhost:8090".parse()?;
        let rpc_full_url = "http://localhost:8545".parse()?;

        let provider = ProviderBuilder::new().on_http(rpc_url);
        let provider_full = ProviderBuilder::new().on_http(rpc_full_url);

        let signer =
            PrivateKeySigner::from_str("0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80").unwrap();
        let wallet = EthereumWallet::from(signer.clone());
        let mut current_nonce = provider_full.get_transaction_count(signer.address()).await?;

        loop {
            let bob = Address::from_str("0xF07208aB3090856D8444F2682a6ecd12832a2944").unwrap();
            let tx = TransactionRequest::default()
                .with_to(bob)
                .with_nonce(current_nonce)
                .with_chain_id(63)
                .with_value(U256::from(100))
                .with_gas_limit(21_000)
                .with_max_priority_fee_per_gas(1_000_000_000)
                .max_fee_per_gas(20_000_000_000);

            let tx_envelope = tx.build(&wallet).await?;
            let tx_encoded = tx_envelope.encoded_2718();

            match provider.send_raw_transaction(&tx_encoded).await {
                Ok(tx_hash) => {
                    println!("Transaction sent successfully: {:?}", tx_hash);
                }
                Err(e) => {
                    eprintln!("Error sending transaction: {:?}", e);
                }
            };

            current_nonce += 1;

            // if (current_nonce % 10) == 0 {
            //     current_nonce = provider_full.get_transaction_count(signer.address()).await?;
            // }

            tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
        }

        Ok(())
    }
}
