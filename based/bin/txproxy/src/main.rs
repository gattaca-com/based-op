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