mod chain;
mod config;
mod docker;
mod engine;
mod types;
mod utils;

use std::time::Duration;

use alloy::{
    eips::BlockNumberOrTag,
    providers::{Provider, ProviderBuilder},
    rpc::types::engine::ForkchoiceState,
};
use clap::Parser;
use op_alloy_network::Optimism;

use crate::{
    config::Args,
    docker::{start_based_gatway, start_based_op_geth_service},
    engine::EngineClient,
    types::execution_payload_from_block,
    utils::{ensure_chain_folder, read_jwt_file},
};

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    let args = Args::parse().validate();

    ensure_chain_folder(args.chain_name).expect("ensure chain folder is configured");

    let jwt_secret = read_jwt_file(args.chain_name).expect("to read jwt file");
    let rpc_client = ProviderBuilder::<_, _, Optimism>::default().connect_http(args.l2_el_rpc_url);

    let replay_block = rpc_client
        .get_block_by_number(BlockNumberOrTag::Number(*args.blocks_range.start()))
        .full()
        .await
        .expect("get replay block")
        .expect("to find block");

    let replay_block_hash = replay_block.hash();
    let parent_beacon_block_root = replay_block.header.parent_beacon_block_root;

    start_based_op_geth_service(args.chain_name).expect("to start based op service");
    let sleep_time = Duration::from_secs(5);
    println!("Waiting {sleep_time:?} for engine to be up...");
    tokio::time::sleep(sleep_time).await;
    let auth_el_client = EngineClient::new(args.l2_engine_rpc_url, jwt_secret);

    let execution_payload = execution_payload_from_block(replay_block);
    let is_at_least_v3 = execution_payload.as_v3().is_some();

    let highest_known_block = auth_el_client
        .get_block_number()
        .await
        .expect("to get block number from engine");
    println!("Current block number of local geth: {highest_known_block}");

    let should_sync = highest_known_block < *args.blocks_range.start();

    if should_sync {
        println!("Sending new payload to based-op-geth, so we trigger sync next");
        let status = auth_el_client
            .new_payload(execution_payload, parent_beacon_block_root)
            .await
            .expect("call new payload");
        println!("New payload status: {status:?}");
    } else {
        println!("based-op-geth is already synced, rolling back the chain via FCU");
    }

    println!("Sending fork choice update with starting block to replay");
    let fcs = ForkchoiceState {
        head_block_hash: replay_block_hash,
        safe_block_hash: replay_block_hash,
        finalized_block_hash: replay_block_hash,
    };
    let update = auth_el_client
        .fork_choice_update(fcs, None, is_at_least_v3)
        .await
        .expect("to make fcu");
    println!("Fork choice update result: {update:?}");

    if should_sync {
        println!("Checking whether based-op-geth is synced");
        let poll_time = Duration::from_secs(5);
        auth_el_client
            .wait_for_sync(poll_time)
            .await
            .expect("make sync status calls");

        println!("based-op-geth is now synced! Starting gateway in verification mode");
    }
    start_based_gatway(args.chain_name).expect("to start gateway");
}
