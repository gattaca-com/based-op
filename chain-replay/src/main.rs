mod chain;
mod config;
mod docker;
mod utils;

use alloy::providers::{Provider, ProviderBuilder};
use clap::Parser;

use crate::{config::Args, docker::start_based_op_service, utils::ensure_chain_folder};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + 'static>> {
    let args = Args::parse().validate();

    ensure_chain_folder(args.chain_name)?;

    let provider = ProviderBuilder::new().connect_http(args.l2_el_rpc_url);
    let sync_target_hash = provider
        .get_block_by_number(alloy::eips::BlockNumberOrTag::Number(
            *args.blocks_range.start(),
        ))
        .await?
        .expect("to find block")
        .header
        .hash;
    println!(
        "Starting based-op-geth with sync target hash: {:?}",
        sync_target_hash
    );

    start_based_op_service(args.chain_name, sync_target_hash)?;

    Ok(())
}
