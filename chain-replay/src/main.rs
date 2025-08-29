mod chain;
mod config;
mod docker;
mod engine;
mod types;
mod utils;

use alloy::{eips::BlockNumberOrTag, providers::Provider};
use clap::Parser;

use crate::{
    config::Args,
    docker::start_based_op_service,
    engine::EngineClient,
    types::execution_payload_v4_from_block,
    utils::{ensure_chain_folder, read_jwt_file},
};

#[tokio::main]
async fn main() {
    let args = Args::parse().validate();

    ensure_chain_folder(args.chain_name).expect("ensure chain folder is configured");

    let jwt_secret = read_jwt_file(args.chain_name).expect("to read jwt file");
    let auth_el_client = EngineClient::new(args.l2_el_rpc_url, jwt_secret);

    let replay_block = auth_el_client
        .get_block_by_number(BlockNumberOrTag::Number(*args.blocks_range.start()))
        .await
        .expect("get replay block")
        .expect("to find block");

    start_based_op_service(args.chain_name).expect("to start based op service");

    let parent_beacon_blocok_root = replay_block
        .header
        .parent_beacon_block_root
        .expect("parent_beacon_blocok_root");
    let execution_payload_v4 = execution_payload_v4_from_block(replay_block);

    let _ = auth_el_client
        .new_payload_v4(execution_payload_v4, parent_beacon_blocok_root)
        .await
        .expect("call new payload v4");
}
