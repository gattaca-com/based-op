use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use bop_common::utils::init_tracing;
use clap::Parser;
use cli::PortalArgs;
use server::PortalServer;
use tracing::info;

mod cli;
mod middleware;
mod server;
mod clients;

#[tokio::main]
async fn main() -> eyre::Result<()> {
    let args = PortalArgs::parse();
    let _guard = init_tracing((&args).into());

    let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), args.portal_port);
    let server = PortalServer::new(args.clone()).await?;

    info!(%addr, registry_url = %args.registry_url, fallback_url = %args.fallback_url, fallback_eth_url = %args.fallback_eth_url, "starting Based Portal");

    server.run(addr).await
}
