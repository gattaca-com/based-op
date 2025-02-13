use std::net::{IpAddr, SocketAddr};

use bop_common::utils::init_tracing;
use clap::Parser;
use cli::PortalArgs;
use server::PortalServer;
use tracing::info;

mod cli;
mod middleware;
mod server;

#[tokio::main]
async fn main() -> eyre::Result<()> {
    let args = PortalArgs::parse();
    let _guard = init_tracing((&args).into());

    let addr = SocketAddr::new(IpAddr::V4(args.portal_host), args.portal_port);
    let server = PortalServer::new(args.clone())?;

    info!(%addr, gateway_url = %args.gateway_url, fallback_url = %args.fallback_url, "starting Based Portal");

    server.run(addr).await
}
