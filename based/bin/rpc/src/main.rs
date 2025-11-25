use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use bop_common::utils::init_tracing;
use clap::Parser;
use cli::RpcArgs;
mod cli;

#[tokio::main]
async fn main() -> eyre::Result<()> {
    let args = RpcArgs::parse();
    let _guard = init_tracing((&args).into());

    let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), args.port);

    todo!()
}
