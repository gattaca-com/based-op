use std::sync::Arc;
use std::time::Duration;

use alloy::{
    eips::BlockNumberOrTag,
    providers::{Provider, ProviderBuilder},
    rpc::types::engine::ForkchoiceState,
};
use kona_engine::EngineClient;
use kona_genesis::RollupConfig;
use op_alloy_network::Optimism;
use tracing::info;
use url::Url;

use crate::{
    chain::ensure_chain_folder,
    config::Args,
    docker::{
        ensure_all_containers_down, start_based_gatway, start_based_op_geth_service,
        start_based_op_node_service, start_based_registry,
    },
    engine::EngineExt as _,
    rpc::{debug_set_head, wait_for_sync},
    types::execution_payload_envelope_from_block,
    utils::read_jwt_file,
};

pub async fn spin_up_follower_nodes(args: Args) {
    ensure_chain_folder(args.chain_name).expect("ensure chain folder is configured");
    info!("Ensuring all containers are down...");
    ensure_all_containers_down(args.chain_name).expect("ensure all containers are down");

    let jwt_secret = read_jwt_file(args.chain_name).expect("to read jwt file");
    let rpc_client =
        ProviderBuilder::<_, _, Optimism>::default().connect_http(args.l2_el_verifier_url.clone());

    let replay_block = rpc_client
        .get_block_by_number(BlockNumberOrTag::Number(*args.blocks_range.start()))
        .full()
        .await
        .expect("get replay block")
        .expect("to find block");

    let replay_block_hash = replay_block.hash();
    let parent_beacon_block_root = replay_block.header.parent_beacon_block_root;

    start_based_op_geth_service(args.chain_name).expect("to start based op service");
    let sleep_time = Duration::from_secs(3);
    info!("Waiting {sleep_time:?} for engine to be up...");
    tokio::time::sleep(sleep_time).await;
    let auth_el_client = EngineClient::new_http(
        args.l2_engine_rpc_url.clone(),
        Url::parse("http://0.0.0.0:1234").unwrap(), // NOTE: we don't use the L1
        Arc::new(RollupConfig::default()),          // FIXME: use actual config
        jwt_secret,
    );

    let execution_payload_envelope = execution_payload_envelope_from_block(replay_block);
    let execution_payload = execution_payload_envelope.execution_payload;
    let is_at_least_v3 = execution_payload.as_v3().is_some();

    let highest_known_block =
        auth_el_client.get_block_number().await.expect("to get block number from engine");
    info!("Current block number of local geth: {highest_known_block}");

    let should_sync = highest_known_block < *args.blocks_range.start();

    if should_sync {
        info!("Sending new payload to based-op-geth, so we trigger sync next");
        let status = auth_el_client
            .new_payload(execution_payload, parent_beacon_block_root)
            .await
            .expect("call new payload");
        info!("New payload status: {status:?}");
    } else {
        info!("based-op-geth is already synced, rolling back the chain via debug_setHead + FCU");
        debug_set_head(auth_el_client.l2_engine(), *args.blocks_range.start())
            .await
            .expect("to roll back");
    }

    info!("Sending fork choice update with starting block to replay");
    let fcs = ForkchoiceState {
        head_block_hash: replay_block_hash,
        safe_block_hash: replay_block_hash,
        finalized_block_hash: replay_block_hash,
    };
    let update =
        auth_el_client.fork_choice_update(fcs, None, is_at_least_v3).await.expect("to make fcu");
    info!("Fork choice update result: {update:?}");

    if should_sync {
        info!("Checking whether based-op-geth is synced");
        let poll_time = Duration::from_secs(5);
        wait_for_sync(auth_el_client.l2_engine(), poll_time).await.expect("make sync status calls");

        info!("based-op-geth is now synced!");
    }

    info!("Starting based-registry");
    start_based_registry(args.chain_name).expect("to start based-registry");

    info!("Starting based-op-node");
    start_based_op_node_service(args.chain_name).expect("to start based-op-node");

    info!("Starting based-gateway");
    start_based_gatway(args).expect("to start gateway");
}
