use std::{fmt, net::SocketAddr, sync::Arc, time::Duration};

use alloy_primitives::{Address, B256, Bytes, U256};
use alloy_rpc_types::{
    BlockId, BlockNumberOrTag,
    engine::{ExecutionPayloadV3, ForkchoiceState, ForkchoiceUpdated, PayloadId, PayloadStatus},
};
use bop_common::{
    api::{
        EngineApiClient, EngineApiServer, EthApiClient, EthApiServer, OpGethAdminApiClient, OpNodeApiClient,
        OpNodeP2PApiClient, OpRpcBlock, PORTAL_CAPABILITIES, PortalApiServer, RegistryApiClient,
    },
    communication::messages::{RpcError, RpcResult},
    time::Nanos,
    utils::{uuid, wait_for_signal},
};
use jsonrpsee::{
    core::{ClientError, async_trait},
    http_client::{HttpClientBuilder, transport::HttpBackend},
    server::{RpcServiceBuilder, ServerBuilder},
};
use op_alloy_rpc_types::OpTransactionReceipt;
use op_alloy_rpc_types_engine::{OpExecutionPayloadEnvelopeV3, OpPayloadAttributes};
use parking_lot::RwLock;
use reqwest::Url;
use reth_rpc_layer::{AuthClientLayer, AuthClientService, JwtSecret};
use tokio::sync::Mutex;
use tracing::{Instrument, Level, debug, error, info, trace};

use crate::{cli::PortalArgs, middleware::ProxyService};

pub type RpcClient = jsonrpsee::http_client::HttpClient;
pub type AuthRpcClient = jsonrpsee::http_client::HttpClient<AuthClientService<HttpBackend>>;

#[derive(Clone)]
struct Gateway {
    id: Url,
    client: AuthRpcClient,
}

impl fmt::Debug for Gateway {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.id)
    }
}

/// If we get fcus faster than this threshold we assume that we are in sync mode and will
/// not propagate anything to the gateways
const SYNC_FCU_DT_THRESHOLD: Nanos = Nanos::from_millis(999);

#[derive(Clone)]
pub struct PortalServer {
    fallback_eth_client: RpcClient,
    fallback_client: AuthRpcClient,
    op_node_client: RpcClient,
    registry_client: RpcClient,
    current_gateway: Arc<Mutex<Option<Gateway>>>,
    gateway_timeout: Duration,
    gateways: Arc<RwLock<Vec<Gateway>>>,
    last_fcu: Arc<RwLock<Nanos>>,
    args: Arc<PortalArgs>,
}

impl PortalServer {
    pub async fn new(args: PortalArgs) -> eyre::Result<Self> {
        let fallback_jwt = args.fallback_jwt();

        let fallback_eth_client =
            create_client(args.fallback_eth_url.clone(), Duration::from_millis(args.fallback_timeout_ms))?;

        let op_node_client = create_client(args.op_node_url.clone(), Duration::from_millis(args.fallback_timeout_ms))?;

        let fallback_client = create_auth_client(
            args.fallback_url.clone(),
            fallback_jwt,
            Duration::from_millis(args.fallback_timeout_ms),
        )?;
        let registry_client =
            create_client(args.registry_url.clone(), Duration::from_millis(args.registry_timeout_ms))?;

        let gateway_timeout = Duration::from_millis(args.gateway_timeout_ms);

        let current_gateway = Arc::new(Mutex::new(registry_client.current_gateway().await.ok().and_then(
            |(_, gateway_url, _, jwt_as_b256)| {
                create_gateway_client(
                    gateway_url,
                    unsafe {
                        std::mem::transmute::<alloy_primitives::FixedBytes<32>, reth_rpc_layer::JwtSecret>(jwt_as_b256)
                    },
                    gateway_timeout,
                )
                .ok()
            },
        )));

        let mut gateways = vec![];
        for (gateway_url, _, jwt_as_b256) in registry_client.registered_gateways().await.unwrap_or_else(|_| vec![]) {
            gateways.push(create_gateway_client(
                gateway_url,
                unsafe {
                    std::mem::transmute::<alloy_primitives::FixedBytes<32>, reth_rpc_layer::JwtSecret>(jwt_as_b256)
                },
                gateway_timeout,
            )?)
        }

        let gateways = Arc::new(RwLock::new(gateways));

        Ok(Self {
            fallback_eth_client,
            fallback_client,
            op_node_client,
            registry_client,
            current_gateway,
            gateways,
            gateway_timeout,
            last_fcu: Default::default(),
            args: Arc::new(args),
        })
    }

    pub async fn run(self, addr: SocketAddr) -> eyre::Result<()> {
        let fallback_client = self.fallback_client.clone();
        let fallback_eth_client = self.fallback_eth_client.clone();
        let op_node_client = self.op_node_client.clone();
        let registry_client = self.registry_client.clone();

        let rpc_middleware = RpcServiceBuilder::new().layer_fn(move |s| {
            ProxyService::new(
                PORTAL_CAPABILITIES,
                s,
                fallback_eth_client.clone(),
                fallback_client.clone(),
                op_node_client.clone(),
                registry_client.clone(),
            )
        });

        let server = ServerBuilder::default()
            .max_request_body_size(u32::MAX)
            .max_response_body_size(u32::MAX)
            .set_rpc_middleware(rpc_middleware)
            .build(addr)
            .await?;

        let mut module = EngineApiServer::into_rpc(self.clone());
        module.merge(EthApiServer::into_rpc(self.clone())).expect("failed to merge modules");
        module.merge(PortalApiServer::into_rpc(self.clone())).expect("failed to merge modules");

        tokio::spawn(async move {
            loop {
                let _ = self.refresh().await;
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        });
        let server_handle = server.start(module);

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

    fn gateways(&self) -> Vec<Gateway> {
        self.gateways.read().clone()
    }

    fn syncing(&self) -> bool {
        self.last_fcu.read().elapsed() < SYNC_FCU_DT_THRESHOLD
    }

    pub async fn refresh(&self) -> eyre::Result<()> {
        let mut gateways = vec![];
        for (gateway_url, _, jwt_as_b256) in self.registry_client.registered_gateways().await? {
            let Ok(client) = create_gateway_client(
                gateway_url,
                unsafe {
                    std::mem::transmute::<alloy_primitives::FixedBytes<32>, reth_rpc_layer::JwtSecret>(jwt_as_b256)
                },
                self.gateway_timeout,
            ) else {
                continue;
            };
            gateways.push(client);
        }
        *self.gateways.write() = gateways;

        let (_, gateway_url, _, _) = self.registry_client.current_gateway().await?;
        for g in self.gateways() {
            if g.id == gateway_url {
                *self.current_gateway.lock().await = Some(g);
                return Ok(());
            }
        }

        error!(
            "CRITICAL: Couldn't find the current gateway in the list we got from the registry. This means the registry is inconsistent"
        );
        Ok(())
    }

    async fn send_fcu(
        fork_choice_state: ForkchoiceState,
        payload_attributes: Option<OpPayloadAttributes>,
        gateway: Gateway,
    ) {
        match gateway.client.fork_choice_updated_v3(fork_choice_state, payload_attributes).await {
            Ok(res) => {
                if res.is_valid() {
                    trace!(?gateway, ?res, "gateway response");
                } else {
                    trace!(?gateway, ?res, "Error: gateway response");
                }
            }
            Err(err) => trace!(%err, "Error: failed gateway"),
        }
        debug!(?gateway, "served fcu")
    }
}

/// This is a temporary API to broacast transactions to both gateway and fallback. In practice this should not be
/// receiving user facing calls so we need to find another way to do this
#[async_trait]
impl EthApiServer for PortalServer {
    #[tracing::instrument(skip_all, err, ret(level = Level::DEBUG), fields(req_id = %uuid()))]
    async fn send_raw_transaction(&self, bytes: Bytes) -> RpcResult<B256> {
        // send to gateways and fallback
        for gateway in self.gateways() {
            let bytes = bytes.clone();
            tokio::spawn(async move {
                if let Err(err) = gateway.client.send_raw_transaction(bytes).await {
                    error!(%err, ?gateway, "failed to send to gateway");
                }
            });
        }

        let response = self.fallback_eth_client.send_raw_transaction(bytes).await?;
        Ok(response)
    }

    #[tracing::instrument(skip_all, err, ret(level = Level::TRACE))]
    async fn transaction_receipt(&self, hash: B256) -> RpcResult<Option<OpTransactionReceipt>> {
        debug!(%hash, "new request");

        let fallback_fut = tokio::spawn(
            {
                let client = self.fallback_client.clone();
                async move { client.transaction_receipt(hash).await }
            }
            .in_current_span(),
        );

        let Some(current_gateway) = self.current_gateway.lock().await.clone() else { return Ok(fallback_fut.await??) };

        let gateway_fut =
            tokio::spawn({ async move { current_gateway.client.transaction_receipt(hash).await } }.in_current_span());

        let (fallback, gateway) = tokio::join!(fallback_fut, gateway_fut);
        // ignore join errors
        let fallback = fallback?;
        let gateway = gateway?;

        let payload = gateway.or(fallback)?;

        Ok(payload)
    }

    #[tracing::instrument(skip_all, err, ret(level = Level::TRACE))]
    async fn block_by_number(&self, number: BlockNumberOrTag, full: bool) -> RpcResult<Option<OpRpcBlock>> {
        debug!(%number, full, "new request");

        let fallback_fut = tokio::spawn(
            {
                let client = self.fallback_client.clone();
                async move { client.block_by_number(number, full).await }
            }
            .in_current_span(),
        );

        let Some(current_gateway) = self.current_gateway.lock().await.clone() else { return Ok(fallback_fut.await??) };

        let gateway_fut = tokio::spawn(
            { async move { current_gateway.client.block_by_number(number, full).await } }.in_current_span(),
        );

        let (fallback, gateway) = tokio::join!(fallback_fut, gateway_fut);
        // ignore join errors
        let fallback = fallback?;
        let gateway = gateway?;

        let payload = gateway.or(fallback)?;

        Ok(payload)
    }

    #[tracing::instrument(skip_all, err, ret(level = Level::TRACE))]
    async fn block_by_hash(&self, hash: B256, full: bool) -> RpcResult<Option<OpRpcBlock>> {
        debug!(%hash, full, "new request");

        let fallback_fut = tokio::spawn(
            {
                let client = self.fallback_client.clone();
                async move { client.block_by_hash(hash, full).await }
            }
            .in_current_span(),
        );
        let Some(current_gateway) = self.current_gateway.lock().await.clone() else { return Ok(fallback_fut.await??) };
        let gateway_fut =
            tokio::spawn({ async move { current_gateway.client.block_by_hash(hash, full).await } }.in_current_span());

        let (fallback, gateway) = tokio::join!(fallback_fut, gateway_fut);
        // ignore join errors
        let fallback = fallback?;
        let gateway = gateway?;

        let payload = gateway.or(fallback)?;

        Ok(payload)
    }

    #[tracing::instrument(skip_all, err, ret(level = Level::TRACE))]
    async fn block_number(&self) -> RpcResult<U256> {
        debug!("block number request");

        let fallback_fut = tokio::spawn(
            {
                let client = self.fallback_client.clone();
                async move { client.block_number().await }
            }
            .in_current_span(),
        );
        let Some(current_gateway) = self.current_gateway.lock().await.clone() else { return Ok(fallback_fut.await??) };
        let gateway_fut =
            tokio::spawn({ async move { current_gateway.client.block_number().await } }.in_current_span());

        let (fallback, gateway) = tokio::join!(fallback_fut, gateway_fut);
        // ignore join errors
        let fallback = fallback?;
        let gateway = gateway?;

        let payload = gateway.or(fallback)?;

        Ok(payload)
    }

    #[tracing::instrument(skip_all, err, ret(level = Level::TRACE))]
    async fn transaction_count(&self, address: Address, block_number: Option<BlockId>) -> RpcResult<U256> {
        debug!(%address, ?block_number, "new request");

        let fallback_fut = tokio::spawn(
            {
                let client = self.fallback_client.clone();
                async move { client.transaction_count(address, block_number).await }
            }
            .in_current_span(),
        );
        let Some(current_gateway) = self.current_gateway.lock().await.clone() else { return Ok(fallback_fut.await??) };
        let gateway_fut = tokio::spawn(
            { async move { current_gateway.client.transaction_count(address, block_number).await } }.in_current_span(),
        );

        let (fallback, gateway) = tokio::join!(fallback_fut, gateway_fut);
        // ignore join errors
        let fallback = fallback?;
        let gateway = gateway?;

        let payload = gateway.or(fallback)?;

        Ok(payload)
    }

    #[tracing::instrument(skip_all, err, ret(level = Level::TRACE))]
    async fn balance(&self, address: Address, block_number: Option<BlockId>) -> RpcResult<U256> {
        debug!(%address, ?block_number, "new request");

        let fallback_fut = tokio::spawn(
            {
                let client = self.fallback_client.clone();
                async move { client.balance(address, block_number).await }
            }
            .in_current_span(),
        );
        let Some(current_gateway) = self.current_gateway.lock().await.clone() else { return Ok(fallback_fut.await??) };
        let gateway_fut = tokio::spawn(
            { async move { current_gateway.client.balance(address, block_number).await } }.in_current_span(),
        );

        let (fallback, gateway) = tokio::join!(fallback_fut, gateway_fut);
        // ignore join errors
        let fallback = fallback?;
        let gateway = gateway?;

        let payload = gateway.or(fallback)?;

        Ok(payload)
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
        let last_fcu_dt = self.last_fcu.read().elapsed();
        if payload_attributes.is_none() {
            *self.last_fcu.write() = Nanos::now();
        }
        let parent_block_hash = fork_choice_state.head_block_hash;

        if let Some(payload_attributes) = payload_attributes.as_ref() {
            let no_tx_pool = payload_attributes.no_tx_pool.unwrap_or(false);
            let gas_limit = payload_attributes.gas_limit.unwrap_or(0);
            debug!(parent_block_hash = %parent_block_hash, no_tx_pool = %no_tx_pool, gas_limit = %gas_limit, "new request (with attributes)");
        } else {
            debug!(%parent_block_hash, "new request (no attributes)");
        }

        let response =
            self.fallback_client.fork_choice_updated_v3(fork_choice_state, payload_attributes.clone()).await?;

        if payload_attributes.is_none() && last_fcu_dt < SYNC_FCU_DT_THRESHOLD {
            debug!("we seem to be in state syncing so only sending fcu to fallback");
            return Ok(response);
        }

        if let Some(current_gateway) = self.current_gateway.as_ref().lock().await.clone() {
            if payload_attributes.is_some() {
                // pick only one gateway for this block
                tokio::spawn(Self::send_fcu(fork_choice_state, payload_attributes, current_gateway).in_current_span());
            } else {
                // send to all gateways
                for gateway in self.gateways() {
                    let payload_attributes = payload_attributes.clone();
                    tokio::spawn(Self::send_fcu(fork_choice_state, payload_attributes, gateway).in_current_span());
                }
            }
        }

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

        let response = self
            .fallback_client
            .new_payload_v3(payload.clone(), versioned_hashes.clone(), parent_beacon_block_root)
            .await?;

        if self.syncing() {
            return Ok(response);
        }

        // send to all gateways
        for gateway in self.gateways() {
            let payload = payload.clone();
            let versioned_hashes = versioned_hashes.clone();

            tokio::spawn(
                async move {
                    match gateway.client.new_payload_v3(payload, versioned_hashes, parent_beacon_block_root).await {
                        Ok(res) => {
                            if res.is_valid() {
                                debug!(?gateway, ?res, "gateway response");
                            } else {
                                error!(?gateway, ?res, "gateway response");
                            }
                        }
                        Err(ClientError::Call(_)) => {}
                        Err(err) => error!(?gateway, %err, "failed gateway"),
                    }
                }
                .in_current_span(),
            );
        }

        Ok(response)
    }

    #[tracing::instrument(skip_all, err, ret(level = Level::DEBUG), fields(req_id = %uuid()))]
    async fn get_payload_v3(&self, payload_id: PayloadId) -> RpcResult<OpExecutionPayloadEnvelopeV3> {
        debug!(%payload_id, "new request");
        if self.syncing() {
            error!("syncing");
            return Ok(self.fallback_client.clone().get_payload_v3(payload_id).await?);
        }

        let fallback_fut = tokio::spawn({
            let client = self.fallback_client.clone();

            async move { client.get_payload_v3(payload_id).await }
        });

        let Some(gateway) = self.current_gateway.lock().await.clone() else { return Ok(fallback_fut.await??) };

        let gateway_fut: tokio::task::JoinHandle<Result<OpExecutionPayloadEnvelopeV3, _>> = tokio::spawn(
            {
                // only get payload from previously picked gateway
                let fallback_client = self.fallback_client.clone();

                async move {
                    let gateway_payload = gateway
                        .client
                        .get_payload_v3(payload_id)
                        .await
                        .inspect_err(|err| error!(%err, "failed gateway"))?;

                    let payload_status = fallback_client
                        .new_payload_v3(
                            gateway_payload.execution_payload.clone(),
                            vec![],
                            gateway_payload.parent_beacon_block_root,
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
                gateway.execution_payload.payload_inner.payload_inner.block_number,
                self.current_gateway.lock().await.as_ref().unwrap()
            );
        } else if let Ok(fallback) = fallback.as_ref() {
            info!(
                "block {}: successfully served from fallback",
                fallback.execution_payload.payload_inner.payload_inner.block_number
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
        Ok(self.fallback_eth_client.node_info().await.map(|p| p.enode)?)
    }
}

fn create_client(url: Url, timeout: Duration) -> eyre::Result<RpcClient> {
    let client = HttpClientBuilder::default()
        .max_request_size(u32::MAX)
        .max_response_size(u32::MAX)
        .request_timeout(timeout)
        .build(url)?;
    Ok(client)
}

fn create_auth_client(url: Url, jwt: JwtSecret, timeout: Duration) -> eyre::Result<AuthRpcClient> {
    let secret_layer = AuthClientLayer::new(jwt);
    let middleware = tower::ServiceBuilder::default().layer(secret_layer);

    let client = HttpClientBuilder::default()
        .max_request_size(u32::MAX)
        .max_response_size(u32::MAX)
        .set_http_middleware(middleware)
        .request_timeout(timeout)
        .build(url)?;

    Ok(client)
}

fn create_gateway_client(url: Url, jwt: JwtSecret, timeout: Duration) -> eyre::Result<Gateway> {
    let client = create_auth_client(url.clone(), jwt, timeout)?;
    let gateway_client = Gateway { client, id: url };
    Ok(gateway_client)
}
