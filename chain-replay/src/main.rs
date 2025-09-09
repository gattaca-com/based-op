#![warn(unused_crate_dependencies)]

mod chain;
mod config;
mod docker;
mod driver;
mod engine;
mod rpc;
mod spawner;
mod types;
mod utils;

use clap::Parser;
use tracing::{info, warn};
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt as _};

use crate::{config::Args, driver::start_kona_sequencer_node, spawner::spin_up_follower_nodes};

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    let _ = tracing_subscriber::registry()
        .with(EnvFilter::from_env("RUST_LOG"))
        .with(tracing_subscriber::fmt::layer().with_ansi(true))
        .try_init();
    let args = Args::parse().validate();

    run(args).await.expect("failed to run chain replication");
}

pub async fn run(args: Args) -> eyre::Result<()> {
    info!("Starting chain replication for blocks {:?}", args.blocks_range);

    spin_up_follower_nodes(args.clone()).await?;
    start_kona_sequencer_node(args).await?;

    Ok(())
}
