use std::{
    fs,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    ops::RangeInclusive,
    str::FromStr as _,
    sync::Arc,
    time::Duration,
};

use alloy::{
    providers::{Provider as _, ProviderBuilder, RootProvider},
    rpc::types::engine::JwtSecret,
};
use eyre::Context as _;
use kona_disc::LocalNode;
use kona_engine::EngineClient;
use kona_genesis::RollupConfig;
use kona_gossip::P2pRpcRequest;
use kona_node_service::{
    NetworkActor, NetworkConfig, NetworkContext, NetworkInboundData, NodeActor,
};
use kona_sources::BlockSigner;
use libp2p::{
    Multiaddr,
    futures::future::join_all,
    identity::secp256k1::{self, SecretKey},
};
use op_alloy_network::Optimism;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};
use url::Url;

use crate::{config::Args, engine::EngineExt, types::execution_payload_envelope_from_block};

pub struct Clients {
    based_op_geth: RootProvider<Optimism>,
    l2_el_verifier: RootProvider<Optimism>,
    gateway: RootProvider<Optimism>,
    gateway_auth: EngineClient,
}

pub fn create_clients(
    based_op_geth_url: Url,
    l2_el_verifier_url: Url,
    gateway_url: Url,
    gateway_auth_url: Url,
    gateway_auth_jwt: JwtSecret,
    rollup_config: RollupConfig,
) -> eyre::Result<Clients> {
    let based_op_geth =
        ProviderBuilder::<_, _, Optimism>::default().connect_http(based_op_geth_url);
    let l2_el_verifier =
        ProviderBuilder::<_, _, Optimism>::default().connect_http(l2_el_verifier_url);
    let gateway = ProviderBuilder::<_, _, Optimism>::default().connect_http(gateway_url);

    let gateway_auth = EngineClient::new_http(
        gateway_auth_url,
        Url::from_str("http://0.0.0.0:1234")?, // NOTE: we don't use the L1
        Arc::new(rollup_config),
        gateway_auth_jwt,
    );

    let clients = Clients { based_op_geth, l2_el_verifier, gateway, gateway_auth };

    Ok(clients)
}

pub async fn start_kona_node(args: Args) -> eyre::Result<()> {
    info!("🎈✣ Starting Kona Sequencer node\n\n");

    let rollup_config_string = fs::read_to_string(args.chain_name.rollup_file_path())?;
    let rollup_config: RollupConfig = serde_json::from_str(&rollup_config_string)?;

    let based_op_geth_port_string =
        std::env::var("BOP_REPLAY_BASED_OP_GETH_RPC_PORT").wrap_err("based-op-geth port set")?;
    let based_op_geth_port =
        u64::from_str(based_op_geth_port_string.trim()).wrap_err("valid based-op-geth port")?;
    let based_op_geth_url = Url::from_str(&format!("http://0.0.0.0:{based_op_geth_port}"))
        .wrap_err("valid based-op-geth url")?;

    let clients = create_clients(
        based_op_geth_url,
        args.l2_el_verifier_url,
        args.gateway_url,
        args.gateway_auth_url,
        args.gateway_auth_jwt,
        rollup_config.clone(),
    )
    .wrap_err("failed to create clients")?;

    let gossip = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), args.gossip_port);
    info!(target: "gossip", "Starting gossip driver on {:?}", gossip);

    let mut gossip_addr = Multiaddr::from(gossip.ip());
    gossip_addr.push(libp2p::multiaddr::Protocol::Tcp(gossip.port()));

    let disc_ip = Ipv4Addr::UNSPECIFIED;
    let disc_addr = LocalNode::new(
        args.p2p_sequencer_key.credential().clone(),
        IpAddr::V4(disc_ip),
        args.disc_port,
        args.disc_port,
    );

    let sk = SecretKey::try_from_bytes(args.p2p_sequencer_key.to_bytes().as_mut_slice())
        .expect("valid private key");
    let keypair = secp256k1::Keypair::from(sk).into();

    let gossip_signer = BlockSigner::Local(args.p2p_sequencer_key.clone());

    let network_config = NetworkConfig {
        discovery_address: disc_addr,
        gossip_address: gossip_addr,
        unsafe_block_signer: args.p2p_sequencer_key.address(),
        discovery_config: discv5::ConfigBuilder::new(discv5::ListenConfig::Ipv4 {
            ip: disc_ip,
            port: args.disc_port,
        })
        .build(),
        discovery_interval: Duration::from_secs(1),
        discovery_randomize: None,
        keypair,
        // Messages are sent to libp2p anonymously and checked in OP consensus. If we sign a
        // message, they will get rejected with a `unexpected signature` message.
        // References:
        // * <https://github.com/gattaca-com/based-optimism/blob/1e803a46f9c8ddfcee795cd59c0d638d28cb904e/op-node/p2p/gossip.go#L185>
        // * <https://github.com/libp2p/go-libp2p-pubsub/blob/ab876fc71c34e89a7f0c8f4e361720ca9fa8588a/pubsub.go#L1393-L1423>
        gossip_config: libp2p::gossipsub::ConfigBuilder::default()
            .validation_mode(libp2p::gossipsub::ValidationMode::Anonymous)
            .build()
            .expect("valid config"),
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
    let (network_inbound_data, network) = NetworkActor::new(network_config.into());

    let (unsafe_blocks_tx, mut unsafe_blocks_rx) = tokio::sync::mpsc::channel(1024);

    let mut tasks = Vec::new();

    let handle = tokio::spawn(async {
        network
            .start(NetworkContext {
                blocks: unsafe_blocks_tx,
                cancellation: CancellationToken::new(),
            })
            .await
            .expect("to start network");
    });

    tasks.push(handle);

    let address = args.p2p_sequencer_key.address();
    let signer = network_inbound_data.signer.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            let _ = signer.send(address).await;
        }
    });

    info!("Gossip driver started, receiving blocks.");
    tasks.push(tokio::spawn(async move {
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
    }));

    let expected_peers_count = 1;

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

    info!(target = expected_peers_count, "Waiting until we have enough peers");
    wait_for_peers(&network_inbound_data, expected_peers_count).await;

    info!("Gossiping payload with block number {}", payload.execution_payload.block_number());
    network_inbound_data.gossip_payload_tx.send(payload.clone()).await?;

    tokio::time::sleep(Duration::from_secs(5)).await;

    info!("Sending the same payload also to the gateway");

    // NOTE: we're ignoring possible error responses, because the gateway code return an internal
    // error on purpose.
    // Reference: <https://github.com/gattaca-com/based-op/blob/9585a9044d9abeadce8e4af7eb624e43f8c1e0b8/based/crates/rpc/src/engine.rs#L55-L67>
    //
    // For some reason, after this call the gateway silently dies and restarts.
    let _ = clients
        .gateway_auth
        .new_payload(payload.execution_payload, payload.parent_beacon_block_root)
        .await;

    let p2p_rpc = network_inbound_data.p2p_rpc.clone();

    tasks.push(tokio::spawn(async {
        print_peers(p2p_rpc).await;
    }));

    run_verification_body(clients, args.blocks_range)
        .await
        .wrap_err("failed to run verification")?;

    join_all(tasks).await;
    Ok(())
}

/// Block the current thread until we have the expected number of peers.
pub async fn wait_for_peers(network_inbound: &NetworkInboundData, expected_peers_count: usize) {
    let retry_time = Duration::from_secs(10);
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

pub async fn print_peers(p2p_rpc: mpsc::Sender<P2pRpcRequest>) -> ! {
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

        debug!("Peer dump: {peer_dump:?}");

        tokio::time::sleep(retry_time).await;
    }
}

pub async fn run_verification_body(
    clients: Clients,
    blocks_range: RangeInclusive<u64>,
) -> eyre::Result<()> {
    let transactions_in_attributes = false;
    let no_tx_pool = Some(true);

    for block_num in blocks_range {
        // let head_block_number = self.next_block.saturating_sub(1);
        //
        // let prev_block =
        //     self.executor.block_on(fetch_block(head_block_number, &self.verification_provider));
        // debug!("Fetched previous block {} for newPayload + FCU", prev_block.number());
        //
        // let (new_payload_prev_block, _, fcu) = messages::EngineApi::messages_from_block(
        //     &prev_block,
        //     transactions_in_attributes,
        //     no_tx_pool,
        // );
        //
        // let block_to_build =
        //     self.executor.block_on(fetch_block(self.next_block, &self.verification_provider));
        //
        // debug!("Fetched block {} to replay", block_to_build.number());
        //
        // // WaitingForNewPayload -> WaitingForForkchoiceWithAttributes
        // connections.send(new_payload_prev_block);
        // Duration::from_millis(1000).sleep();
        //
        // let txs_for_pool: Vec<_> = Transaction::from_block(&block_to_build);
        //
        // // WaitingForForkchoiceWithAttributes -> Sorting
        // connections.send(fcu);
        //
        // for t in txs_for_pool {
        //     connections.send(t);
        //     Duration::from_millis(10).sleep();
        // }
        //
        // Duration::from_millis(2000).sleep();
        //
        // let (block_tx, mut block_rx) = oneshot::channel();
        // // Sorting -> WaitingForNewPayload
        // connections
        //     .send(EngineApi::GetPayloadV4 { payload_id: PayloadId::new([0; 8]), res: block_tx });
        // Duration::from_millis(100).sleep();
        // let curt = Instant::now();
        // let mut sealed_block = loop {
        //     if let Ok(sealed_block) = block_rx.try_recv() {
        //         break sealed_block;
        //     }
        //     if curt.elapsed() > Duration::from_secs(2) {
        //         tracing::warn!("couldn't get block");
        //         return;
        //     }
        // };
        //
        // let hash = block_to_build.hash_slow();
        // let hash1 =
        //     sealed_block.execution_payload.payload_inner.payload_inner.payload_inner.block_hash;
        // if hash1 != hash {
        //     sealed_block.execution_payload.payload_inner.payload_inner.payload_inner.transactions =
        //         vec![];
        //     let receipt = sealed_block
        //         .execution_payload
        //         .payload_inner
        //         .payload_inner
        //         .payload_inner
        //         .receipts_root;
        //     if receipt == block_to_build.receipts_root {
        //         info!("receipts match");
        //     } else {
        //         info!(our=%receipt, block = %block_to_build.receipts_root, "receipts don't match");
        //         debug_assert!(false, "receipts don't match");
        //     };
        //
        //     let gas_used =
        //         sealed_block.execution_payload.payload_inner.payload_inner.payload_inner.gas_used;
        //
        //     if gas_used == block_to_build.gas_used() {
        //         info!("gas_used matches")
        //     } else {
        //         info!(our=%gas_used, block = %block_to_build.gas_used(), "gas_used doesn't match");
        //         debug_assert!(false, "gas_used doesn't match");
        //     };
        //
        //     let state_root =
        //         sealed_block.execution_payload.payload_inner.payload_inner.payload_inner.state_root;
        //
        //     if state_root == block_to_build.state_root() {
        //         info!("state_root matches")
        //     } else {
        //         info!(our=%state_root, block = %block_to_build.state_root(), "state_root doesn't match");
        //         debug_assert!(false, "state_root doesn't match");
        //     };
        //
        //     // println!("ACTUAL BLOCK:");
        // }
        //
        // assert_eq!(
        //     sealed_block.execution_payload.payload_inner.payload_inner.payload_inner.block_hash,
        //     block_to_build.hash_slow(),
        //     "{block_to_build:#?} vs {sealed_block:#?}"
        // );
        //
        // // WaitingForNewPayload -> WaitingForForkchoiceWithAttributes
        // // connections.send(new_payload);
        // // connections.send(fcu_1);
        //
        // self.next_block += 1;
    }

    Ok(())
}
