use std::{
    fs, io,
    process::{Command, ExitStatus},
    sync::Arc,
    time::Duration,
};

use alloy::{
    eips::BlockNumberOrTag,
    providers::{Provider, ProviderBuilder},
    rpc::types::engine::ForkchoiceState,
};
use eyre::Context as _;
use kona_engine::EngineClient;
use kona_genesis::RollupConfig;
use op_alloy_network::Optimism;
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
    utils::{extract_stdout, read_jwt_file},
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
    let sleep_time = Duration::from_secs(3);
    info!("Waiting {sleep_time:?} for engine to be up...");
    tokio::time::sleep(sleep_time).await;

    let auth_el_client = EngineClient::new_http(
        args.l2_engine_rpc_url.clone(),
        Url::parse("http://0.0.0.0:1234").unwrap(), // NOTE: we don't use the L1
        Arc::new(rollup_config),
        jwt_secret,
    );

    let execution_payload_envelope = execution_payload_envelope_from_block(block_to_sync);
    let execution_payload = execution_payload_envelope.execution_payload;
    let is_at_least_v3 = execution_payload.as_v3().is_some();

    let highest_known_block =
        auth_el_client.get_block_number().await.wrap_err("get block number from engine")?;
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
    wait_for_gateway_sync(args.chain_name, sync_target_block_number + 1)
        .wrap_err("failed to check gateway sync status")?;
    info!("Gateway synced!");

    Ok(())
}

/// Waits until the "CanonicalHeaders" table in the gateway database has the provided number of
/// entries, which correspond to the desired sync or rollback.
///
/// Requires `op-reth` to available in `$PATH`.
///
/// ### Implementation notes
///
/// The output of the `op-reth db stats` table looks as follows:
///
/// ```text
/// | Table Name                 | # Entries | Branch Pages | Leaf Pages | Overflow Pages | Total Size |
/// |----------------------------|-----------|--------------|------------|----------------|------------|
/// | AccountChangeSets          | 4037      | 2            | 40         | 0              | 168 KiB    |
/// ... ... ... ... ... ... ... ... ... ... ... ... ... ... ... ... ... ... ... ... ... ... ... ... ...
/// | CanonicalHeaders           | 998       | 1            | 14         | 0              | 60 KiB     |
/// ... ... ... ... ... ... ... ... ... ... ... ... ... ... ... ... ... ... ... ... ... ... ... ... ...
/// | -------------------------- | --------- | ------------ | ---------- | -------------- | ---------- |
/// | Tables                     |           |              |            |                | 5 MiB      |
/// | Freelist                   | 2543      |              |            |                | 9.9 MiB    |
/// ```
///
/// So in order to extract the number of entries in the `CanonicalHeaders` table we grep for such
/// table name, we trim whitespaces, and we get the third match with `awk` using
/// `--field-separator=\|`.
///
/// It's a bit hackish, but avoids a lot of boilerplate code needed to just the head block.
pub fn wait_for_gateway_sync(chain_name: ChainName, sync_target: u64) -> io::Result<()> {
    let data_path = chain_name.gateway_data_directory_path();
    let data_path_string = data_path.to_string_lossy();
    let genesis_file_path = chain_name.genesis_file_path();
    let genesis_file_path_string = genesis_file_path.to_string_lossy();

    let mut head = 0;

    let mut is_op_reth_in_path = Command::new("which");
    if !is_op_reth_in_path.arg("op-reth").spawn()?.wait()?.success() {
        panic!("op-reth not found in path. Please download a binary from Reth Github releases");
    }

    let command_str = format!(
        "op-reth db --datadir {data_path_string} --chain {genesis_file_path_string} stats | grep CanonicalHeaders | tr -d \' \' | awk --field-separator=\\| \'{{ print $3 }}\'"
    );

    while head != sync_target {
        let mut command = Command::new("sh");
        command.arg("-c").arg(&command_str);
        let output = command.output()?;

        let stdout = extract_stdout(&command_str, output)?;
        let text = String::from_utf8_lossy(&stdout);
        head =
            (text.trim()).parse::<u64>().unwrap_or_else(|_| panic!("valid u64 str, got: {}", text));
        debug!(gateway_head = head, target = sync_target, "Gateway is syncing...");
    }

    Ok(())
}
