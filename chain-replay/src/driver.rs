use std::{
    fs,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    ops::RangeInclusive,
    str::FromStr as _,
    sync::Arc,
    time::Duration,
};

use alloy::{
    eips::Encodable2718,
    providers::{Provider as _, ProviderBuilder, RootProvider},
    rpc::types::{
        Block,
        engine::{ForkchoiceState, JwtSecret, PayloadId},
    },
    signers::local::PrivateKeySigner,
};
use eyre::Context as _;
use kona_disc::LocalNode;
use kona_engine::EngineClient;
use kona_genesis::RollupConfig;
use kona_gossip::{P2pRpcRequest, default_config_builder};
use kona_node_service::{
    NetworkActor, NetworkConfig, NetworkContext, NetworkInboundData, NodeActor,
};
use kona_sources::BlockSigner;
use libp2p::{
    Multiaddr,
    identity::secp256k1::{self, SecretKey},
};
use op_alloy_network::Optimism;
use op_alloy_provider::ext::engine::OpEngineApi as _;
use op_alloy_rpc_types_engine::{OpExecutionPayload, OpExecutionPayloadEnvelope};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, trace, warn};
use url::Url;

use crate::{
    config::Args,
    docker::start_based_op_node_service,
    engine::EngineExt,
    types::{execution_payload_envelope_from_block, op_attributes_from_block},
};

pub struct Clients {
    based_op_geth: RootProvider<Optimism>,
    l2_el_verifier: RootProvider<Optimism>,
    gateway: RootProvider<Optimism>,
    gateway_auth: EngineClient,
}

impl Clients {
    fn new(
        based_op_geth_url: Url,
        l2_el_verifier_url: Url,
        gateway_url: Url,
        gateway_auth_url: Url,
        gateway_auth_jwt: JwtSecret,
        rollup_config: RollupConfig,
    ) -> Self {
        let based_op_geth =
            ProviderBuilder::<_, _, Optimism>::default().connect_http(based_op_geth_url);
        let l2_el_verifier =
            ProviderBuilder::<_, _, Optimism>::default().connect_http(l2_el_verifier_url);
        let gateway = ProviderBuilder::<_, _, Optimism>::default().connect_http(gateway_url);

        let gateway_auth = EngineClient::new_http(
            gateway_auth_url,
            Url::from_str("http://127.0.0.1:1234").expect("valid URL"), // NOTE: we don't use the L1
            Arc::new(rollup_config),
            gateway_auth_jwt,
        );

        Self { based_op_geth, l2_el_verifier, gateway, gateway_auth }
    }
}

pub async fn start_kona_sequencer_node(args: Args) -> eyre::Result<()> {
    info!("🎈✣ Starting Kona Sequencer node\n\n");

    let rollup_config_string = fs::read_to_string(args.chain_name.rollup_file_path())?;
    let rollup_config: RollupConfig = serde_json::from_str(&rollup_config_string)?;

    let based_op_geth_port_string = std::env::var("BOP_REPLAY_BASED_OP_GETH_0_RPC_PORT")
        .wrap_err("based-op-geth-0 port set")?;
    let based_op_geth_port =
        u64::from_str(based_op_geth_port_string.trim()).wrap_err("valid based-op-geth-0 port")?;
    let based_op_geth_url = Url::from_str(&format!("http://0.0.0.0:{based_op_geth_port}"))
        .wrap_err("valid based-op-geth-0 url")?;

    let clients = Clients::new(
        based_op_geth_url,
        args.l2_el_verifier_url,
        args.gateway_url,
        args.gateway_auth_url,
        args.gateway_auth_jwt,
        rollup_config.clone(),
    );

    let (network_inbound_data, network) = init_network_actor(
        args.p2p_sequencer_key.clone(),
        args.gossip_port,
        args.disc_port,
        rollup_config,
    )
    .wrap_err("failed to initaliaze network")?;

    let (unsafe_blocks_tx, mut unsafe_blocks_rx) = tokio::sync::mpsc::channel(1024);

    tokio::spawn(async {
        network
            .start(NetworkContext {
                blocks: unsafe_blocks_tx,
                cancellation: CancellationToken::new(),
            })
            .await
            .expect("to start network");
    });
    info!("Gossip driver started, receiving blocks.");

    // NOTE: we start bop-node here so that dialing the kona sequencer as static peers works
    // immediately and we don't have to wait one minute for the next attempt.
    tokio::time::sleep(Duration::from_secs(1)).await;
    for instance_num in 0..args.follower_nodes_count {
        info!("Starting based-op-node-{instance_num}");
        start_based_op_node_service(args.chain_name, instance_num, args.follower_nodes_count)
            .wrap_err("to start based-op-node")?;
    }

    tokio::spawn(async move {
        loop {
            match unsafe_blocks_rx.recv().await {
                Some(block) => {
                    info!(target: "gossip", "Received unsafe block: {:?}", block);
                }
                None => {
                    warn!(target: "gossip", "unsafe block gossip channel closed");
                }
            }
        }
    });

    info!(target = args.follower_nodes_count, "Waiting until we have enough peers");
    wait_for_peers(&network_inbound_data, args.follower_nodes_count as usize).await;

    // NOTE: it is N-1 because N will be inserted via frags.
    let first_block_to_gossip = clients
        .l2_el_verifier
        .get_block_by_number(alloy::eips::BlockNumberOrTag::Number(
            args.blocks_range.start().saturating_sub(1),
        ))
        .full()
        .await?
        .expect("to find block");
    let payload = execution_payload_envelope_from_block(first_block_to_gossip);

    info!("Gossiping payload with block number {}", payload.execution_payload.block_number());
    network_inbound_data.gossip_payload_tx.send(payload.clone()).await?;

    info!("Sending the same payload also to the gateway");

    // NOTE: we're ignoring possible error responses, because the gateway code return an internal
    // error on purpose.
    // Reference: <https://github.com/gattaca-com/based-op/blob/9585a9044d9abeadce8e4af7eb624e43f8c1e0b8/based/crates/rpc/src/engine.rs#L55-L67>
    //
    // For some reason, after this call the gateway silently dies and restarts.
    //
    // WaitingForNewPayload -> WaitingForForkchoiceWithAttributes
    let _ = clients
        .gateway_auth
        .new_payload(payload.execution_payload, payload.parent_beacon_block_root)
        .await;

    let p2p_rpc = network_inbound_data.p2p_rpc.clone();

    tokio::spawn(async {
        print_peers(p2p_rpc).await;
    });

    run_verification_body(&clients, &network_inbound_data, args.blocks_range, args.fcu_wait_ms)
        .await
        .wrap_err("failed to run verification")?;

    Ok(())
}

/// Init the network stack from provided configurable, and returns an handle to send and receive
/// commands, and a task handle.
fn init_network_actor(
    p2p_sequencer_key: PrivateKeySigner,
    gossip_port: u16,
    disc_port: u16,
    rollup_config: RollupConfig,
) -> eyre::Result<(NetworkInboundData, NetworkActor)> {
    let gossip = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), gossip_port);
    info!(target: "gossip", "Starting gossip driver on {:?}", gossip);

    let mut gossip_addr = Multiaddr::from(gossip.ip());
    gossip_addr.push(libp2p::multiaddr::Protocol::Tcp(gossip.port()));

    let disc_ip = Ipv4Addr::UNSPECIFIED;
    let disc_addr = LocalNode::new(
        p2p_sequencer_key.credential().clone(),
        IpAddr::V4(disc_ip),
        disc_port,
        disc_port,
    );

    let sk = SecretKey::try_from_bytes(p2p_sequencer_key.to_bytes().as_mut_slice())
        .expect("valid private key");
    let keypair = secp256k1::Keypair::from(sk).into();

    let gossip_signer = BlockSigner::Local(p2p_sequencer_key.clone());

    let network_config = NetworkConfig {
        discovery_address: disc_addr,
        gossip_address: gossip_addr,
        unsafe_block_signer: p2p_sequencer_key.address(),
        discovery_config: discv5::ConfigBuilder::new(discv5::ListenConfig::Ipv4 {
            ip: disc_ip,
            port: disc_port,
        })
        .build(),
        discovery_interval: Duration::from_secs(1),
        discovery_randomize: None,
        keypair,
        gossip_config: default_config_builder().build().expect("valid default config"),
        scoring: Default::default(),
        topic_scoring: Default::default(),
        monitor_peers: Default::default(),
        bootstore: None,
        gater_config: Default::default(),
        bootnodes: Vec::new(),
        rollup_config: rollup_config.clone(),
        gossip_signer: Some(gossip_signer),
        enr_update: true,
    };
    Ok(NetworkActor::new(network_config.into()))
}

/// Block the current thread until we have the expected number of peers.
async fn wait_for_peers(network_inbound: &NetworkInboundData, expected_peers_count: usize) {
    let retry_time = Duration::from_secs(5);
    loop {
        let (tx, rx) = oneshot::channel();
        let peer_count_request = P2pRpcRequest::PeerCount(tx);
        if network_inbound.p2p_rpc.send(peer_count_request).await.is_err() {
            error!("Failed to send peer count request: channel closed. Retrying in {retry_time:?}");
            tokio::time::sleep(retry_time).await;
        };

        let Ok((discv5_peer_count, gossip_peer_count)) = rx.await else {
            error!("Failed to receive peer count info: channel closed. Retrying in {retry_time:?}");
            tokio::time::sleep(retry_time).await;
            continue;
        };

        debug!("discv5_peer_count, {discv5_peer_count:?}");

        if gossip_peer_count >= expected_peers_count {
            info!(
                ?gossip_peer_count,
                target = expected_peers_count,
                "We have enough peers to start chain replication"
            );
            break;
        }

        info!(
            ?gossip_peer_count,
            target = expected_peers_count,
            "Waiting for more peers. Re-checking in {retry_time:?}"
        );
        tokio::time::sleep(retry_time).await;
    }
}

/// Routine to periodically dump the number of peers we have.
async fn print_peers(p2p_rpc: mpsc::Sender<P2pRpcRequest>) -> ! {
    let retry_time = Duration::from_secs(10);
    loop {
        let (tx, rx) = oneshot::channel();
        let peers_request = P2pRpcRequest::Peers { out: tx, connected: true };
        if p2p_rpc.send(peers_request).await.is_err() {
            error!("Failed to send peer count request: channel closed. Retrying in {retry_time:?}");
            tokio::time::sleep(retry_time).await;
        };

        let Ok(peer_dump) = rx.await else {
            error!("Failed to receive peer count info: channel closed. Retrying in {retry_time:?}");
            tokio::time::sleep(retry_time).await;
            continue;
        };

        trace!("Peer dump: {peer_dump:?}");

        tokio::time::sleep(retry_time).await;
    }
}

/// The actual chain replication logic. It iters through the block numbers specified in the
/// configured range, and sends all transactions within a certain block to the gateway, along with
/// Engine API messages to drive it through its various states.
///
/// After the gateway seals a block, retrieved via a `GetPayload` call, it is compared with the
/// block from the chain we're targeting. In case of block hash mismatch, an helper function is
/// called to show differences between the two.
async fn run_verification_body(
    clients: &Clients,
    network_inbound: &NetworkInboundData,
    blocks_range: RangeInclusive<u64>,
    fcu_wait: u64,
) -> eyre::Result<()> {
    for block_num in blocks_range {
        info!("Replaying block {block_num}");

        let (prev_block, block) = tokio::join!(
            clients.l2_el_verifier.get_block_by_number(block_num.saturating_sub(1).into()).full(),
            clients.l2_el_verifier.get_block_by_number(block_num.into()).full()
        );
        let Some(prev_block) = prev_block? else {
            return Err(eyre::eyre!("Block {} not found", block_num.saturating_sub(1)));
        };
        let Some(block) = block? else {
            return Err(eyre::eyre!("Block {} not found", block_num));
        };
        let block_hash = block.hash();

        let system_tx = block
            .transactions
            .first_transaction()
            .cloned()
            .expect("all OP blocks should have at least the system tx");

        let raw_txs = block
            .transactions
            .clone()
            .into_transactions()
            // Skip L2 deposit info system transaction.
            .skip(1)
            .filter_map(|t| {
                if t.inner.inner.is_system_transaction() {
                    None
                } else {
                    Some(t.inner.inner.into_inner().encoded_2718())
                }
            })
            .collect::<Vec<_>>();

        let raw_txs_len = raw_txs.len();
        let fcs = ForkchoiceState { head_block_hash: prev_block.header.hash, ..Default::default() };
        let op_attributes = op_attributes_from_block(&block, system_tx);

        // WaitingForForkchoiceWithAttributes -> Sorting
        // NOTE: we don't check error here because of the no response policy from the gateway.
        let _ = clients.gateway_auth.fork_choice_update(fcs, Some(op_attributes), true).await;

        // NOTE: Ensure FCU message is processed, so that transactions are received only when the
        // Gateway is in Sorting state.
        tokio::time::sleep(Duration::from_millis(fcu_wait)).await;

        // NOTE: we send all the transactions and we make a get payload call immediately after,
        // leading to a variable block time. Enforcing a 2s timeout can slow down the test
        // unnecessarily and on large blocks it can result in not sending all the transactions in
        // time.
        for t in raw_txs {
            let _ = clients.gateway.send_raw_transaction(&t).await.expect("to send tx to gateway");
        }
        debug!("Sent {raw_txs_len} transactions to gateway for block {block_num}");

        // Adds a small delay to make sure last every transaction is processed, and not end up
        // with testing failure because of a couple of missing transactions.
        tokio::time::sleep(Duration::from_millis(5)).await;

        // Sorting -> WaitingForNewPayload
        let sealed_block = clients.gateway_auth.get_payload_v4(PayloadId::default()).await?;

        // WaitingForNewPayload -> WaitingForForkchoiceWithAttributes
        network_inbound
            .gossip_payload_tx
            .send(OpExecutionPayloadEnvelope {
                execution_payload: OpExecutionPayload::V4(sealed_block.execution_payload.clone()),
                parent_beacon_block_root: Some(sealed_block.parent_beacon_block_root),
            })
            .await?;
        // NOTE: we don't check error here because of the no response policy from the gateway.
        let _ = clients
            .gateway_auth
            .new_payload(
                OpExecutionPayload::V4(sealed_block.execution_payload.clone()),
                Some(sealed_block.parent_beacon_block_root),
            )
            .await;

        // Check if there is a match!

        let sealed_block_inner =
            &sealed_block.execution_payload.payload_inner.payload_inner.payload_inner;
        if sealed_block_inner.block_hash != block_hash {
            error!(
                "Found block hash mismatch for block {block_num}: sealed = {:?}, target = {:?}",
                sealed_block_inner.block_hash, block_hash
            );
            // Our follower node should have this block in a reasonable time-frame
            let mut attempts = 0;
            let based_op_geth_block = loop {
                let Some(block) =
                    clients.based_op_geth.get_block_by_number(block_num.into()).full().await?
                else {
                    if attempts == 20 {
                        panic!("failed to fetch sealed block {block_num} from bop-geth");
                    } else {
                        attempts += 1;
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        continue;
                    }
                };
                break block;
            };
            show_block_mismatches(&based_op_geth_block, &block);
            panic!(
                "Block mismatch:\n\nour    = {:?}\n\ntarget = {:?}",
                based_op_geth_block.hash(),
                block.hash()
            );
        }
    }

    info!("CHAIN REPLICATION COMPLETED SUCCESFULLY!");

    std::process::exit(0);
}

/// Compares two blocks and prints mismatches found.
fn show_block_mismatches(
    produced_block: &Block<op_alloy_rpc_types::Transaction>,
    target: &Block<op_alloy_rpc_types::Transaction>,
) {
    if produced_block.header.parent_hash != target.header.parent_hash {
        error!(
            "Parent hash mismatch: produced={:?}, target={:?}",
            produced_block.header.parent_hash, target.header.parent_hash
        );
    }

    if produced_block.header.number != target.header.number {
        error!(
            "Block number mismatch: produced={}, target={}",
            produced_block.header.number, target.header.number
        );
    }

    if produced_block.header.timestamp != target.header.timestamp {
        error!(
            "Timestamp mismatch: produced={}, target={}",
            produced_block.header.timestamp, target.header.timestamp
        );
    }

    if produced_block.header.gas_limit != target.header.gas_limit {
        error!(
            "Gas limit mismatch: produced={}, target={}",
            produced_block.header.gas_limit, target.header.gas_limit
        );
    }

    if produced_block.header.gas_used != target.header.gas_used {
        error!(
            "Gas used mismatch: produced={}, target={}",
            produced_block.header.gas_used, target.header.gas_used
        );
    }

    if produced_block.header.base_fee_per_gas != target.header.base_fee_per_gas {
        error!(
            "Base fee per gas mismatch: produced={:?}, target={:?}",
            produced_block.header.base_fee_per_gas, target.header.base_fee_per_gas
        );
    }

    if produced_block.header.state_root != target.header.state_root {
        error!(
            "State root mismatch: produced={:?}, target={:?}",
            produced_block.header.state_root, target.header.state_root
        );
    }

    if produced_block.header.receipts_root != target.header.receipts_root {
        error!(
            "Receipts root mismatch: produced={:?}, target={:?}",
            produced_block.header.receipts_root, target.header.receipts_root
        );
    }

    if produced_block.header.logs_bloom != target.header.logs_bloom {
        error!(
            "Logs bloom mismatch: produced={:?}, target={:?}",
            produced_block.header.logs_bloom, target.header.logs_bloom
        );
    }

    if produced_block.header.extra_data != target.header.extra_data {
        error!(
            "Extra data mismatch: produced={:?}, target={:?}",
            produced_block.header.extra_data, target.header.extra_data
        );
    }

    // Optimism-specific header fields
    if produced_block.header.withdrawals_root != target.header.withdrawals_root {
        error!(
            "Withdrawals root mismatch: produced={:?}, target={:?}",
            produced_block.header.withdrawals_root, target.header.withdrawals_root
        );
    }

    if produced_block.header.parent_beacon_block_root != target.header.parent_beacon_block_root {
        error!(
            "Parent beacon block root mismatch: produced={:?}, target={:?}",
            produced_block.header.parent_beacon_block_root, target.header.parent_beacon_block_root
        );
    }

    // Transaction count comparison
    if produced_block.transactions.len() != target.transactions.len() {
        error!(
            "Transaction count mismatch: produced={}, target={}",
            produced_block.transactions.len(),
            target.transactions.len()
        );
    }

    let max_len = produced_block.transactions.len().max(target.transactions.len());

    let produced_block_txs_slice = produced_block.transactions.as_transactions().unwrap();
    let target_block_txs_slice = target.transactions.as_transactions().unwrap();

    let produced_and_target_pairs = (0..max_len)
        .map(|i| (produced_block_txs_slice.get(i).cloned(), target_block_txs_slice.get(i).cloned()))
        .collect::<Vec<_>>();

    // Individual transaction comparisons
    for (i, (produced_tx, target_tx)) in produced_and_target_pairs.iter().enumerate() {
        let produced_tx_hash = produced_tx.as_ref().map(|t| t.inner.inner.hash());
        let target_tx_hash = target_tx.as_ref().map(|t| t.inner.inner.hash());
        if produced_tx_hash != target_tx_hash {
            error!(
                "Transaction hash mismatch at index {}: produced={:?}, target={:?}",
                i, produced_tx_hash, target_tx_hash
            );
        } else {
            debug!(
                "Transaction hash match at index {}: produced={:?}, target={:?}",
                i, produced_tx_hash, target_tx_hash
            );
        }
    }

    // Withdrawals comparison (if present)
    match (&produced_block.withdrawals, &target.withdrawals) {
        (Some(produced_withdrawals), Some(target_withdrawals)) => {
            if produced_withdrawals.len() != target_withdrawals.len() {
                error!(
                    "Withdrawals count mismatch: produced={}, target={}",
                    produced_withdrawals.len(),
                    target_withdrawals.len()
                );
            }

            for (i, (produced_w, target_w)) in
                produced_withdrawals.iter().zip(target_withdrawals.iter()).enumerate()
            {
                if produced_w != target_w {
                    error!(
                        "Withdrawal mismatch at index {}: produced={:?}, target={:?}",
                        i, produced_w, target_w
                    );
                }
            }
        }
        (Some(_), None) => error!("Produced block has withdrawals, target block does not"),
        (None, Some(_)) => error!("Target block has withdrawals, produced block does not"),
        (None, None) => {} // Both blocks have no withdrawals, which is fine
    }
}
