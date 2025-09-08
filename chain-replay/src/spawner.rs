use std::{fs, io, sync::Arc, time::Duration};

use alloy::{
    eips::BlockNumberOrTag,
    providers::{Provider, ProviderBuilder},
    rpc::types::engine::ForkchoiceState,
};
use alloy_genesis::Genesis;
use eyre::Context as _;
use kona_engine::EngineClient;
use kona_genesis::RollupConfig;
use op_alloy_network::Optimism;
use reth_db::open_db_read_only;
use reth_optimism_chainspec::OpChainSpec;
use reth_optimism_node::OpNode;
use reth_provider::{BlockNumReader, providers::StaticFileProvider};
use tracing::{debug, info};
use url::Url;

use crate::{
    chain::{ChainName, ensure_chain_folder},
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

/// Responsible for spinning up all follower nodes via docker:
///
/// - based-op-geth;
/// - based-op-node;
/// - based-gateway.
///
/// Moreover, it ensure all of them are synced and eventually rolled back to the required state to
/// being chain replication.
pub async fn spin_up_follower_nodes(args: Args) -> eyre::Result<()> {
    ensure_chain_folder(args.chain_name).wrap_err("ensure chain folder is configured")?;

    info!("Ensuring all containers are down...");
    ensure_all_containers_down(args.chain_name).wrap_err("ensure all containers are down")?;

    let rollup_config_string = fs::read_to_string(args.chain_name.rollup_file_path())?;
    let rollup_config: RollupConfig = serde_json::from_str(&rollup_config_string)?;

    let jwt_secret = read_jwt_file(args.chain_name).wrap_err("to read jwt file")?;

    let rpc_client =
        ProviderBuilder::<_, _, Optimism>::default().connect_http(args.l2_el_verifier_url.clone());

    // NOTE: we need to sync to N-2 because N-1 must be sent as UnsafeL2Payload from a sequencer
    // node, and N will be the first block produced via frags.
    let sync_target_block_number = args.blocks_range.start().saturating_sub(2);

    let genesis_block = rpc_client
        .get_block_by_number(BlockNumberOrTag::Number(0))
        .await
        .wrap_err("get genesis block")?
        .expect("to find some block");
    info!("Genesis block hash: {:?}", genesis_block.hash());
    let block_to_sync = rpc_client
        .get_block_by_number(BlockNumberOrTag::Number(sync_target_block_number))
        .full()
        .await
        .wrap_err("get replay block")?
        .expect("to find some block");
    let block_to_sync_hash = block_to_sync.hash();
    let parent_beacon_block_root = block_to_sync.header.parent_beacon_block_root;

    start_based_op_geth_service(args.chain_name).wrap_err("to start based op service")?;
    let auth_el_client = EngineClient::new_http(
        args.l2_engine_rpc_url.clone(),
        Url::parse("http://0.0.0.0:1234").unwrap(), // NOTE: we don't use the L1
        Arc::new(rollup_config),
        jwt_secret,
    );

    info!("Waiting for engine to be up...");
    let highest_known_block = wait_for_based_op_geth_up(&auth_el_client).await?;

    let execution_payload_envelope = execution_payload_envelope_from_block(block_to_sync);
    let execution_payload = execution_payload_envelope.execution_payload;
    let is_at_least_v3 = execution_payload.as_v3().is_some();

    info!("Current block number of local geth: {highest_known_block}");

    let should_sync = highest_known_block < sync_target_block_number;

    if should_sync {
        info!("Sending new payload to based-op-geth, so we trigger sync next");
        let status = auth_el_client
            .new_payload(execution_payload, parent_beacon_block_root)
            .await
            .wrap_err("call new payload")?;
        info!("New payload status: {status:?}");
    } else {
        info!("based-op-geth is already synced, rolling back the chain via debug_setHead + FCU");
        debug_set_head(auth_el_client.l2_engine(), sync_target_block_number)
            .await
            .wrap_err("to roll back")?;
    }

    info!("Sending fork choice update with starting block to replay");
    let fcs = ForkchoiceState {
        head_block_hash: block_to_sync_hash,
        // NOTE: it might cause sequencer drift issues? Investigate
        safe_block_hash: genesis_block.hash(),
        finalized_block_hash: genesis_block.hash(),
    };
    let update = auth_el_client
        .fork_choice_update(fcs, None, is_at_least_v3)
        .await
        .wrap_err("to make fcu")?;
    info!("Fork choice update result: {update:?}");

    if should_sync {
        info!("Checking whether based-op-geth is synced");
        let poll_time = Duration::from_secs(5);
        wait_for_sync(auth_el_client.l2_engine(), poll_time)
            .await
            .wrap_err("make sync status calls")?;

        info!("based-op-geth is now synced!");
    }

    info!("Starting based-registry");
    start_based_registry(args.chain_name).wrap_err("to start based-registry")?;

    info!("Starting based-op-node");
    start_based_op_node_service(args.chain_name).wrap_err("to start based-op-node")?;

    info!("Starting based-gateway");
    start_based_gatway(args.clone()).wrap_err("to start gateway")?;

    info!("Waiting for gateway to sync");
    // For some weird rollback commitments reason, we provide +1 here.
    wait_for_gateway_sync(args.chain_name, sync_target_block_number)
        .await
        .wrap_err("failed to check gateway sync status")?;
    info!("Gateway synced!");

    Ok(())
}

/// Waits until the gateway is synced by opening a read-only connection to the Reth database and
/// querying it.
pub async fn wait_for_gateway_sync(chain_name: ChainName, sync_target: u64) -> io::Result<()> {
    let data_path = chain_name.gateway_data_directory_path();
    let genesis_file_path = chain_name.genesis_file_path();
    let genesis_json_string = fs::read_to_string(genesis_file_path)?;

    let genesis: Genesis = serde_json::from_str(&genesis_json_string)?;
    let chain_spec = OpChainSpec::from_genesis(genesis);

    let db_path = data_path.join("db");
    let static_files_dir = data_path.join("static_files");

    let factory = OpNode::provider_factory_builder()
        .db(Arc::new(open_db_read_only(db_path, Default::default()).expect("to open reth db")))
        .chainspec(Arc::new(chain_spec))
        .static_file(
            StaticFileProvider::read_only(static_files_dir, false).expect("to open static files"),
        )
        .build_provider_factory();

    let mut head = factory.best_block_number().expect("to read best block number from db");
    let sleep_time = Duration::from_secs(5);
    while head != sync_target {
        debug!(
            gateway_head = head,
            target = sync_target,
            "Gateway is syncing, re-checking in {sleep_time:?}..."
        );

        tokio::time::sleep(sleep_time).await;

        head = factory.best_block_number().expect("to read best block number from db");
    }

    Ok(())
}

pub async fn wait_for_based_op_geth_up(auth_el_client: &EngineClient) -> eyre::Result<u64> {
    let attempts = 10;

    let mut e = Ok(());

    for _ in 1..=attempts {
        match auth_el_client.get_block_number().await {
            Ok(block) => return Ok(block),
            Err(err) => {
                e = Err(err);
                tokio::time::sleep(Duration::from_secs(3)).await;
            }
        }
    }

    Err(eyre::eyre!("based-op-geth is still not up after {attempts} attempts: {e:?}"))
}
