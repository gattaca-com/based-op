use std::net::{IpAddr, SocketAddr};

use bop_common::utils::init_tracing;
use clap::Parser;
use cli::PortalArgs;
use server::PortalServer;
use tracing::{info, Level};

mod cli;
mod middleware;
mod server;

#[tokio::main]
async fn main() -> eyre::Result<()> {
    let args = PortalArgs::parse();

    let log_level = if args.trace {
        Level::TRACE
    } else if args.debug {
        Level::DEBUG
    } else {
        Level::INFO
    };

    let _guard = init_tracing(None, 0, Some(vec![&log_level.to_string()]));

    let addr = SocketAddr::new(IpAddr::V4(args.portal_host), args.portal_port);
    let server = PortalServer::new(args.clone())?;

    info!(gateway_url = %args.gateway_url, fallback_url = %args.fallback_url, "starting Based Portal");

    server.run(addr).await
}
