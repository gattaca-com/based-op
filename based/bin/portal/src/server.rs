use std::{
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering::Relaxed},
    },
};

use alloy_eips::eip7685::RequestsOrHash;
use alloy_primitives::B256;
use alloy_rpc_types::engine::{ExecutionPayloadV3, ForkchoiceState, ForkchoiceUpdated, PayloadId, PayloadStatus};
use bop_common::{
    api::{
        EngineApiClient, EngineApiServer, OpGethAdminApiClient, OpMinerExtApiServer, OpNodeApiClient,
        OpNodeP2PApiClient, PortalApiServer,
    },
    communication::messages::{RpcError, RpcResult},
    time::Duration,
    utils::{uuid, wait_for_signal},
};
use jsonrpsee::{
    core::{ClientError, async_trait},
    server::{RpcServiceBuilder, ServerBuilder},
};
use op_alloy_rpc_types_engine::{OpExecutionPayloadEnvelopeV4, OpExecutionPayloadV4, OpPayloadAttributes};
use tokio::sync::RwLock;
use tower::ServiceBuilder;
use tower_http::cors::{Any, CorsLayer};
use tracing::{Instrument, Level, debug, error, info, trace};

use crate::{
    cli::PortalArgs,
    clients::{AuthRpcClient, GatewayManager, RpcClient, create_auth_client, create_client},
    middleware::EngineApiProxy,
};

#[derive(Clone)]
pub struct PortalServer {
    geth_client: RpcClient,
    geth_engine_client: AuthRpcClient,
    op_node_client: RpcClient,

    gateway_manager: Arc<GatewayManager>,

    new_payload_block_hash: Arc<RwLock<B256>>,
    new_payload_block_number: Arc<AtomicU64>,
    current_block_number: Arc<AtomicU64>,
    args: Arc<PortalArgs>,
}

impl PortalServer {
    pub async fn new(args: PortalArgs) -> eyre::Result<Self> {
        let timeout = Duration::from_millis(args.fallback_timeout_ms);
        let fallback_eth_client = create_client(args.fallback_eth_url.clone(), timeout)?;
        let op_node_client = create_client(args.op_node_url.clone(), timeout)?;
        let fallback_client = create_auth_client(args.fallback_url.clone(), args.fallback_jwt(), timeout)?;

        let temp = Self {
            geth_client: fallback_eth_client,
            geth_engine_client: fallback_client,
            op_node_client,
            gateway_manager: Arc::new(GatewayManager::new_from_args(&args)),
            new_payload_block_number: Arc::new(AtomicU64::new(0)),
            new_payload_block_hash: Arc::new(RwLock::new(B256::ZERO)),
            current_block_number: Arc::new(AtomicU64::new(0)),
            args: Arc::new(args),
        };

        let _ = temp.gateway_manager.update_gateway_list().await;

        Ok(temp)
    }

    pub async fn run(self, addr: SocketAddr) -> eyre::Result<()> {
        let geth_engine_client = self.geth_engine_client.clone();
        let registry_client = self.gateway_manager.registry_client.clone();
        let op_node_client = self.op_node_client.clone();

        let rpc_middleware = RpcServiceBuilder::new().layer_fn(move |s| EngineApiProxy {
            inner: s,
            geth_client: geth_engine_client.clone(),
            registry_client: registry_client.clone(),
            op_node_client: op_node_client.clone(),
        });

        // temp: remove when factoring out the portal
        let cors = CorsLayer::new().allow_origin(Any).allow_methods(Any).allow_headers(Any);
        let cors_middleware = ServiceBuilder::new().layer(cors);

        let server = ServerBuilder::default()
            .max_request_body_size(u32::MAX)
            .max_response_body_size(u32::MAX)
            .set_rpc_middleware(rpc_middleware)
            .set_http_middleware(cors_middleware)
            .build(addr)
            .await?;

        let mut module = EngineApiServer::into_rpc(self.clone());
        module.merge(PortalApiServer::into_rpc(self.clone())).expect("failed to merge modules");
        let server_handle = server.start(module);

        let gateway_manager = Arc::clone(&self.gateway_manager);
        tokio::spawn(async move {
            loop {
                if let Err(err) = gateway_manager.update_gateway_list().await {
                    error!(%err, "Failed to fetch registered gateways");
                }
                gateway_manager.health_check().await;
                tokio::time::sleep(Duration::from_secs(1).into()).await;
            }
        });

        tokio::select! {
            _ = server_handle.stopped() => {
                error!("server stopped");
            }

            _ = wait_for_signal() => {
                info!("received signal, shutting down");
            }
        }

        Ok(())
    }

    pub async fn on_new_payload(&self, block_number: u64, block_hash: B256) {
        *self.new_payload_block_hash.write().await = block_hash;
        self.new_payload_block_number.store(block_number, Relaxed);
    }

    pub async fn on_fork_choice_updated(
        &self,
        fork_choice_state: &ForkchoiceState,
        payload_attributes: &Option<OpPayloadAttributes>,
    ) {
        let new_payload_block_hash = *self.new_payload_block_hash.read().await;
        debug!(?fork_choice_state, ?payload_attributes, ?new_payload_block_hash, "on_fork_choice_updated called");

        if payload_attributes.is_some() && new_payload_block_hash == fork_choice_state.head_block_hash {
            let block_number = self.new_payload_block_number.load(Relaxed);
            self.current_block_number.store(block_number, Relaxed);
            self.gateway_manager.decide_current_gateway().await;
        }
    }
}

#[async_trait]
impl EngineApiServer for PortalServer {
    #[tracing::instrument(skip_all, err, ret(level = Level::DEBUG), fields(req_id = %uuid()))]
    async fn fork_choice_updated_v3(
        &self,
        fork_choice_state: ForkchoiceState,
        payload_attributes: Option<OpPayloadAttributes>,
    ) -> RpcResult<ForkchoiceUpdated> {
        let parent_block_hash = fork_choice_state.head_block_hash;

        if let Some(payload_attributes) = payload_attributes.as_ref() {
            let no_tx_pool = payload_attributes.no_tx_pool.unwrap_or(false);
            let gas_limit = payload_attributes.gas_limit.unwrap_or(0);
            debug!(parent_block_hash = %parent_block_hash, no_tx_pool = %no_tx_pool, gas_limit = %gas_limit, "new request (with attributes)");
        } else {
            debug!(%parent_block_hash, "new request (no attributes)");
        }

        self.on_fork_choice_updated(&fork_choice_state, &payload_attributes).await;

        let response =
            self.geth_engine_client.fork_choice_updated_v3(fork_choice_state, payload_attributes.clone()).await?;

        self.gateway_manager.send_fcu(fork_choice_state, payload_attributes).await;

        Ok(response)
    }

    #[tracing::instrument(skip_all, err, ret(level = Level::DEBUG), fields(req_id = %uuid()))]
    async fn new_payload_v4(
        &self,
        payload: OpExecutionPayloadV4,
        versioned_hashes: Vec<B256>,
        parent_beacon_block_root: B256,
        requests: RequestsOrHash,
    ) -> RpcResult<PayloadStatus> {
        let block_number = payload.payload_inner.payload_inner.payload_inner.block_number;
        let block_hash = payload.payload_inner.payload_inner.payload_inner.block_hash;
        let gas_limit = payload.payload_inner.payload_inner.payload_inner.gas_limit;
        let gas_used = payload.payload_inner.payload_inner.payload_inner.gas_used;
        let n_txs = payload.payload_inner.payload_inner.payload_inner.transactions.len();
        let n_withdrawals = payload.payload_inner.payload_inner.withdrawals.len();
        let blob_gas_used = payload.payload_inner.blob_gas_used;
        let excess_blob_gas = payload.payload_inner.excess_blob_gas;

        debug!(block_number, %block_hash, gas_limit, gas_used, n_txs, n_withdrawals, blob_gas_used, excess_blob_gas, "new request");

        self.on_new_payload(block_number, block_hash).await;

        let response = self
            .geth_engine_client
            .new_payload_v4(payload.clone(), versioned_hashes.clone(), parent_beacon_block_root, requests.clone())
            .await
            .inspect_err(|e| tracing::error!("issue sending new_payload_v4 to el {e}"))?;

        self.gateway_manager
            .broadcast_new_payload_v4(payload, versioned_hashes, parent_beacon_block_root, requests)
            .await;

        Ok(response)
    }

    #[tracing::instrument(skip_all, err, ret(level = Level::DEBUG), fields(req_id = %uuid()))]
    async fn new_payload_v3(
        &self,
        payload: ExecutionPayloadV3,
        versioned_hashes: Vec<B256>,
        parent_beacon_block_root: B256,
    ) -> RpcResult<PayloadStatus> {
        let block_number = payload.payload_inner.payload_inner.block_number;
        let block_hash = payload.payload_inner.payload_inner.block_hash;
        let gas_limit = payload.payload_inner.payload_inner.gas_limit;
        let gas_used = payload.payload_inner.payload_inner.gas_used;
        let n_txs = payload.payload_inner.payload_inner.transactions.len();
        let n_withdrawals = payload.payload_inner.withdrawals.len();
        let blob_gas_used = payload.blob_gas_used;
        let excess_blob_gas = payload.excess_blob_gas;

        debug!(block_number, %block_hash, gas_limit, gas_used, n_txs, n_withdrawals, blob_gas_used, excess_blob_gas, "new request");

        self.on_new_payload(block_number, block_hash).await;

        let response = self
            .geth_engine_client
            .new_payload_v3(payload.clone(), versioned_hashes.clone(), parent_beacon_block_root)
            .await
            .inspect_err(|e| tracing::error!("issue sending new_payload_v3 to el {e}"))?;

        self.gateway_manager.broadcast_new_payload_v3(payload, versioned_hashes, parent_beacon_block_root).await;

        Ok(response)
    }

    #[tracing::instrument(skip_all, err, ret(level = Level::DEBUG), fields(req_id = %uuid()))]
    async fn get_payload_v4(&self, payload_id: PayloadId) -> RpcResult<OpExecutionPayloadEnvelopeV4> {
        debug!(%payload_id, "new request");

        let fallback_fut = tokio::spawn({
            let client = self.geth_engine_client.clone();

            async move { client.get_payload_v4(payload_id).await }
        });

        let Some(gateway) = self.gateway_manager.current_gateway().await else { return Ok(fallback_fut.await??) };

        let gateway_fut: tokio::task::JoinHandle<Result<OpExecutionPayloadEnvelopeV4, _>> = tokio::spawn(
            {
                // only get payload from previously picked gateway
                let fallback_client = self.geth_engine_client.clone();

                async move {
                    let gateway_payload = gateway
                        .client
                        .get_payload_v4(payload_id)
                        .await
                        .inspect_err(|err| error!(%err, "failed gateway"))?;

                    let payload_status = fallback_client
                        .new_payload_v4(
                            OpExecutionPayloadV4 {
                                payload_inner: gateway_payload.execution_payload.payload_inner.clone(),
                                withdrawals_root: gateway_payload.execution_payload.withdrawals_root,
                            },
                            vec![],
                            gateway_payload.parent_beacon_block_root,
                            RequestsOrHash::default(),
                        )
                        .await
                        .inspect_err(|err| error!(%err, "failed fallback validation"))?;

                    if payload_status.is_valid() {
                        trace!(?gateway, ?gateway_payload, ?payload_status, "gateway response");
                        Ok(gateway_payload)
                    } else {
                        error!(?gateway, ?gateway_payload, ?payload_status, "gateway response");
                        Err(RpcError::Internal)
                    }
                }
            }
            .in_current_span(),
        );

        let (fallback, gateway) = tokio::join!(fallback_fut, gateway_fut);

        // ignore join errors
        let fallback = fallback?;
        let gateway = gateway?;
        if let Ok(gateway) = gateway.as_ref() {
            info!(
                "block {}: successfully served from based-gateway {:?}",
                gateway.execution_payload.payload_inner.payload_inner.payload_inner.block_number, gateway
            );
        } else if let Ok(fallback) = fallback.as_ref() {
            info!(
                "block {}: successfully served from fallback",
                fallback.execution_payload.payload_inner.payload_inner.payload_inner.block_number
            );
        } else {
            error!("couldn't serve a block from fallback or gateway");
        }

        let payload = gateway.or(fallback)?;
        Ok(payload)
    }
}

#[async_trait]
impl PortalApiServer for PortalServer {
    /// The network id of the l2
    async fn l2_chain_id(&self) -> RpcResult<u64> {
        Ok(self.op_node_client.rollup_config().await.map(|config| config.l2_chain_id)?)
    }

    /// The network id of the l1
    async fn l1_chain_id(&self) -> RpcResult<u64> {
        Ok(self.op_node_client.rollup_config().await.map(|config| config.l1_chain_id)?)
    }

    /// rollup.json file
    async fn file_rollup(&self) -> RpcResult<String> {
        let genesis_path = self.args.config_dir.join("rollup.json");
        Ok(std::fs::read_to_string(genesis_path)?)
    }

    /// genesis.json file
    async fn file_genesis(&self) -> RpcResult<String> {
        let genesis_path = self.args.config_dir.join("genesis.json");
        Ok(std::fs::read_to_string(genesis_path)?)
    }

    /// The gossip static address string used by the op-node
    async fn op_node_gossip_static(&self) -> RpcResult<String> {
        Ok(self.op_node_client.peer_info().await.and_then(|p| {
            p.addresses.last().cloned().map(Ok).unwrap_or(Err(ClientError::Custom("empty peer addresses".to_string())))
        })?)
    }

    /// The enr that can be used to sync with the op-node
    async fn op_node_bootnode_enr(&self) -> RpcResult<String> {
        Ok(self.op_node_client.peer_info().await.map(|p| p.enr)?)
    }

    /// The enode that can be used to sync with the op-geth
    async fn op_geth_bootnode_enode(&self) -> RpcResult<String> {
        Ok(self.geth_client.node_info().await.map(|p| p.enode)?)
    }
}

#[async_trait]
impl OpMinerExtApiServer for PortalServer {
    async fn set_max_da_size(&self, max_tx_size: u64, max_block_size: u64) -> RpcResult<bool> {
        self.gateway_manager.broadcast_set_max_da_size(max_tx_size, max_block_size).await;
        Ok(true)
    }
}
