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
use tracing::warn;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt};

use crate::{config::Args, driver::start_kona_node, spawner::spin_up_follower_nodes};

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    let _ = tracing_subscriber::registry()
        .with(EnvFilter::from_env("RUST_LOG"))
        .with(tracing_subscriber::fmt::layer().with_ansi(true));
    let args = Args::parse().validate();

    run(args).await.expect("failed to run chain replication");
}

pub async fn run(args: Args) -> eyre::Result<()> {
    spin_up_follower_nodes(args.clone()).await;
    start_kona_node(args).await?;

    Ok(())
}
