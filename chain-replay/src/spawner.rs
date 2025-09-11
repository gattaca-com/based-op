use std::{fs, sync::Arc, time::Duration};

use alloy::{
    eips::BlockNumberOrTag,
    providers::{Provider, ProviderBuilder, RootProvider},
    rpc::types::engine::ForkchoiceState,
};
use alloy_genesis::Genesis;
use eyre::Context as _;
use kona_engine::EngineClient;
use kona_genesis::RollupConfig;
use libp2p::futures::future::join_all;
use op_alloy_network::Optimism;
use reth_db::{CanonicalHeaders, cursor::DbCursorRO, open_db_read_only};
use reth_optimism_chainspec::OpChainSpec;
use reth_optimism_node::OpNode;
use reth_provider::providers::StaticFileProvider;
use tracing::{debug, info};
use url::Url;

use crate::{
    chain::{ChainName, ensure_chain_folder},
    config::Args,
    docker::{
        ensure_all_containers_down, start_based_gatway, start_based_op_geth_service,
        start_based_registry, start_monitoring_compose,
    },
    engine::EngineExt as _,
    rpc::{debug_set_head, wait_for_el_on_head},
    types::execution_payload_envelope_from_block,
    utils::{get_ip, read_jwt_file},
};

/// Responsible for spinning up all follower nodes via docker:
///
/// - based-op-geth;
/// - based-op-node;
/// - based-gateway.
///
/// Moreover, it ensure all of them are synced and eventually rolled back to the required state to
/// being chain replication.
///
/// NOTE: in practice, based-op-node isn't spawn here, and the reason is for faster startup times,
/// so that when it starts the dial to the static peer is immediately successful and it is not
/// retried after a minute.
pub async fn spin_up_follower_nodes(args: Args) -> eyre::Result<()> {
    ensure_chain_folder(args.chain_name).wrap_err("ensure chain folder is configured")?;

    info!("Ensuring all containers are down...");
    ensure_all_containers_down(args.chain_name).wrap_err("ensure all containers are down")?;

    let rollup_config_string = fs::read_to_string(args.chain_name.rollup_file_path())?;
    let rollup_config: Arc<RollupConfig> = Arc::new(serde_json::from_str(&rollup_config_string)?);

    let jwt_secret = read_jwt_file(args.chain_name).wrap_err("to read jwt file")?;

    let rpc_client =
        ProviderBuilder::<_, _, Optimism>::default().connect_http(args.l2_el_verifier_url.clone());

    // NOTE: we need to sync to N-2 because N-1 must be sent as UnsafeL2Payload from a sequencer
    // node, and N will be the first block produced via frags.
    let sync_target_block_number = args.blocks_range.start().saturating_sub(2);

    let mut tasks = Vec::new();
    for instance_num in 0..args.follower_nodes_count {
        let rollup_config = rollup_config.clone();
        let rpc_client = rpc_client.clone();
        let task = tokio::spawn(async move {
            info!("Starting based-op-geth-{instance_num}");
            start_based_op_geth_service(args.chain_name, instance_num)
                .expect("to start based op service");
            let env = format!("BOP_REPLAY_BASED_OP_GETH_{instance_num}_ENGINE_RPC_PORT");
            let port = std::env::var(env.clone()).expect(&env);
            let engine_url =
                format!("http://{}:{}", get_ip(), port).parse::<Url>().expect("valid url");
            let auth_el_client = EngineClient::new_http(
                engine_url,
                Url::parse("http://0.0.0.0:1234").unwrap(), // NOTE: we don't use the L1
                rollup_config.clone(),
                jwt_secret,
            );

            wait_for_based_op_geth_sync(
                auth_el_client,
                instance_num,
                rpc_client.clone(),
                sync_target_block_number,
            )
            .await
            .unwrap_or_else(|_| panic!("failed to wait for based-op-geth-{instance_num} sync"));
        });
        tasks.push(task);
    }

    join_all(tasks).await.iter().for_each(|t| {
        if let Err(e) = t {
            panic!("based-op-geth task failed: {e}")
        }
    });

    info!("Starting based-registry");
    start_based_registry(args.chain_name).wrap_err("to start based-registry")?;

    info!("Starting based-gateway");
    start_based_gatway(args.clone()).wrap_err("to start gateway")?;

    info!("Waiting for gateway to sync");
    wait_for_gateway_sync(args.chain_name, sync_target_block_number)
        .await
        .wrap_err("failed to check gateway sync status")?;
    info!("Gateway synced!");

    info!("Starting monitoring services");
    start_monitoring_compose().wrap_err("failed to start monitoring services")?;

    Ok(())
}

/// Wait for based-op-geth to be synced to the specified target, rolling back the chain if
/// necessary.
pub async fn wait_for_based_op_geth_sync(
    based_op_geth: EngineClient,
    instance_num: u8,
    rpc_client: RootProvider<Optimism>,
    sync_target: u64,
) -> eyre::Result<()> {
    let genesis_block = rpc_client
        .get_block_by_number(BlockNumberOrTag::Number(0))
        .await
        .wrap_err("get genesis block")?
        .expect("to find some block");
    info!("Genesis block hash: {:?}", genesis_block.hash());
    let block_to_sync = rpc_client
        .get_block_by_number(BlockNumberOrTag::Number(sync_target))
        .full()
        .await
        .wrap_err("get replay block")?
        .expect("to find some block");

    let block_to_sync_hash = block_to_sync.hash();
    let parent_beacon_block_root = block_to_sync.header.parent_beacon_block_root;

    info!("Waiting for based-op-geth engine {instance_num} to be up...");
    let highest_known_block = wait_for_based_op_geth_up(&based_op_geth).await?;

    let execution_payload_envelope = execution_payload_envelope_from_block(block_to_sync);
    let execution_payload = execution_payload_envelope.execution_payload;
    let is_at_least_v3 = execution_payload.as_v3().is_some();

    info!("Current block number of local based-op-geth-{instance_num}: {highest_known_block}");

    let should_sync = highest_known_block < sync_target;
    let should_rollback = highest_known_block > sync_target;

    if should_sync {
        info!("Sending new payload to based-op-geth-{instance_num}, so we trigger sync next");
        let status = based_op_geth
            .new_payload(execution_payload, parent_beacon_block_root)
            .await
            .wrap_err("call new payload")?;
        info!("New payload status from based-op-geth-{instance_num}: {status:?}");
    } else if should_rollback {
        info!(
            "based-op-geth-{instance_num} is too far ahead on the chain, rolling back the chain via debug_setHead + FCU. This can take some time..."
        );
        // NOTE: this call might take a very long time because rolling back takes time, as such we
        // don't check for errors here.
        let _ = debug_set_head(based_op_geth.l2_engine(), sync_target).await;
    }

    info!(
        "Sending fork choice update to based-op-geth-{instance_num} with starting block to replay. If rolling back, it waits the previous call is completed."
    );
    let fcs = ForkchoiceState {
        head_block_hash: block_to_sync_hash,
        // NOTE: it might cause sequencer drift issues? Investigate
        safe_block_hash: genesis_block.hash(),
        finalized_block_hash: genesis_block.hash(),
    };
    let update = based_op_geth
        .fork_choice_update(fcs, None, is_at_least_v3)
        .await
        .wrap_err("to make fcu")?;
    info!("Fork choice update result: {update:?}");

    if should_sync || should_rollback {
        info!("Checking whether based-op-geth-{instance_num} is at the right head target");
        wait_for_el_on_head(based_op_geth.l2_engine(), instance_num, sync_target)
            .await
            .wrap_err("make sync status calls")?;

        info!("based-op-geth is now synced!");
    }

    Ok(())
}

/// Waits until the gateway is synced by opening a read-only connection to the Reth database and
/// querying it.
pub async fn wait_for_gateway_sync(chain_name: ChainName, sync_target: u64) -> eyre::Result<()> {
    let data_path = chain_name.gateway_data_directory_path();
    let genesis_file_path = chain_name.genesis_file_path();
    let genesis_json_string = fs::read_to_string(genesis_file_path)?;

    let genesis: Genesis = serde_json::from_str(&genesis_json_string)?;
    let chain_spec = Arc::new(OpChainSpec::from_genesis(genesis));

    let db_path = data_path.join("db");
    let static_files_dir = data_path.join("static_files");

    loop {
        let factory = OpNode::provider_factory_builder()
            .db(Arc::new(
                open_db_read_only(db_path.clone(), Default::default()).expect("to open reth db"),
            ))
            .chainspec(chain_spec.clone())
            .static_file(
                StaticFileProvider::read_only(static_files_dir.clone(), false)
                    .expect("to open static files"),
            )
            .build_provider_factory();

        let provider = factory.provider()?;

        let latest_header = {
            let mut binding = provider.tx_ref().new_cursor::<CanonicalHeaders>()?;
            let walker = binding.walk(None);
            let res = walker.into_iter().last().unwrap();
            let (number, _hash) = res.last().unwrap()?;
            number
        };

        if latest_header == sync_target {
            break;
        }

        let sleep_time = Duration::from_secs(10);
        debug!(
            gateway_head = latest_header,
            target = sync_target,
            "Gateway is syncing, re-checking in {sleep_time:?}..."
        );
        tokio::time::sleep(sleep_time).await;
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
