use std::net::{IpAddr, SocketAddr};

use bop_common::utils::init_tracing;
use clap::Parser;
use cli::MuxArgs;
use server::MuxServer;
use tracing::{info, Level};

mod cli;
mod middleware;
mod server;

#[tokio::main]
async fn main() -> eyre::Result<()> {
    let args = MuxArgs::parse();

    let log_level = if args.trace {
        Level::TRACE
    } else if args.debug {
        Level::DEBUG
    } else {
        Level::INFO
    };

    let _guard = init_tracing(None, 0, Some(vec![&log_level.to_string()]));

    let addr = SocketAddr::new(IpAddr::V4(args.mux_host), args.mux_port);
    let server = MuxServer::new(args.clone())?;

    info!(gateway_url = %args.gateway_url, fallback_url = %args.fallback_url, "starting Sequencer MUX");

    server.run(addr).await
}
