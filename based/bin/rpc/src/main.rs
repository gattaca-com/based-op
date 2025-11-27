use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    str::FromStr,
    thread,
    time::Duration,
};

use alloy_primitives::Address;
use alloy_provider::{Provider, ProviderBuilder, WsConnect};
use bop_common::{
    p2p::VersionedMessage,
    utils::{init_tracing, wait_for_signal},
};
use clap::Parser;
use cli::RpcArgs;
use jsonrpsee::{
    server::{ServerBuilder, ServerConfigBuilder},
    ws_client::RpcServiceBuilder,
};
use op_alloy_network::Optimism;
use reqwest::Url;
use tokio::time::interval;
use tower::ServiceBuilder;
use tower_http::cors::{Any, CorsLayer};
use tracing::{debug, error, info, warn};

use crate::{
    listener::{spawn_block_listener, spawn_receipt_listener_frag_stream},
    middleware::EthApiProxy,
    server::{Server, create_client},
    types::EthApiServer,
};

mod cli;
mod listener;
mod middleware;
mod server;
mod types;
mod unsealed_block;

#[tokio::main(flavor = "multi_thread", worker_threads = 10)]
async fn main() -> eyre::Result<()> {
    let args = RpcArgs::parse();
    let _guard = init_tracing((&args).into());

    let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), args.port);

    let (block_tx, block_rx) = crossbeam_channel::bounded(100);
    let (message_tx, message_rx) = crossbeam_channel::bounded(100);

    let eth_ws_url = args.eth_ws_url.clone();
    let provider_with_filler = ProviderBuilder::<_, _, Optimism>::default()
        .connect_ws(WsConnect::new(Url::parse(&eth_ws_url).unwrap()))
        .await
        .expect("failed to connect to eth rpc");

    let provider = provider_with_filler.root();

    spawn_receipt_listener_frag_stream(args.frag_url.as_str(), message_tx);
    spawn_block_listener(provider.clone(), block_tx);

    let tx_receiver_provider = match args.tx_receiver_url {
        Some(url) => {
            let parsed_url = Url::parse(&url).expect("invalid tx receiver url");
            let provider_with_filler = match parsed_url.scheme() {
                "ws" | "wss" => ProviderBuilder::<_, _, Optimism>::default()
                    .connect_ws(WsConnect::new(parsed_url))
                    .await
                    .expect("failed to connect to tx receiver via ws"),
                "http" | "https" => ProviderBuilder::<_, _, Optimism>::default().connect_http(parsed_url),
                _ => panic!("unsupported URL scheme for tx receiver: {}", parsed_url.scheme()),
            };
            provider_with_filler.root().clone()
        }
        None => provider.clone(),
    };

    let server_obj = Server::new(provider.clone(), tx_receiver_provider);

    let server = server_obj.clone();
    thread::spawn(move || {
        loop {
            let mut should_sleep = true;

            while let Ok(msg) = message_rx.try_recv() {
                match msg.message {
                    VersionedMessage::FragV0(frag) => {
                        debug!("got frag: block number {} seq {}", frag.block_number, frag.seq);
                        server.on_frag(frag, msg.state_update);
                    }
                    VersionedMessage::SealV0(seal) => {
                        debug!("got seal: block number {}", seal.block_number);
                        server.on_seal(seal);
                        if msg.state_update.is_some() {
                            error!("seal message should not contain state update");
                        }
                    }
                    VersionedMessage::EnvV0(env) => {
                        debug!("got env: block number {}", env.number);
                        server.on_env(env);
                        if msg.state_update.is_some() {
                            error!("env message should not contain state update");
                        }
                    }
                    _ => {
                        warn!("unsupported message type: {:?}", msg.message);
                        if msg.state_update.is_some() {
                            error!("unsupported message type should not contain state update");
                        }
                    }
                }
                should_sleep = false;
            }

            while let Ok(header) = block_rx.try_recv() {
                debug!("got block header: block number {}", header.number);
                server.on_header(header);
                should_sleep = false;
            }

            if should_sleep {
                thread::sleep(Duration::from_millis(1));
            }
        }
    });

    let server = server_obj.clone();
    tokio::spawn(async move {
        let address_to_check = Address::from_str("0x4D36DE6a194dDF98EE57323CfA3A45351d35e442").unwrap();
        let mut interval = interval(Duration::from_secs_f64(0.1));
        loop {
            let transaction_count = server.get_transaction_count(address_to_check).await.unwrap();
            let balance = server.get_balance(address_to_check).await.unwrap();
            let block_number = server.block_number().await.unwrap();
            info!("block number: {} count: {} balance: {:?}", block_number, transaction_count, balance);
            interval.tick().await;
        }
    });

    // temp: remove when factoring out the portal
    let cors = CorsLayer::new().allow_origin(Any).allow_methods(Any).allow_headers(Any);
    let cors_middleware = ServiceBuilder::new().layer(cors);

    let rpc_middleware = RpcServiceBuilder::new().layer_fn(move |s| EthApiProxy {
        inner: s,
        geth_client: create_client(Url::parse(args.eth_http_url.as_str()).unwrap(), Duration::from_secs(2)).unwrap(),
    });

    let rpc_server = ServerBuilder::default()
        .set_config(ServerConfigBuilder::new().max_request_body_size(u32::MAX).max_response_body_size(u32::MAX).build())
        .set_rpc_middleware(rpc_middleware)
        .set_http_middleware(cors_middleware)
        .build(addr)
        .await?;

    let module = EthApiServer::into_rpc(server_obj);
    let server_handle = rpc_server.start(module);

    tokio::select! {
        _ = server_handle.stopped() => {
            error!("server stopped");
        }

        _ = wait_for_signal() => {
            info!("received signal, shutting down");
        }
    }

    Ok(())
}
