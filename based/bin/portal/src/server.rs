use std::{
    fmt,
    net::SocketAddr,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

use alloy_primitives::{Bytes, B256};
use alloy_rpc_types::engine::{ExecutionPayloadV3, ForkchoiceState, ForkchoiceUpdated, PayloadId, PayloadStatus};
use bop_common::{
    api::{EngineApiClient, EngineApiServer, EthApiClient, MinimalEthApiServer, CAPABILITIES},
    communication::messages::{RpcError, RpcResult},
    utils::{uuid, wait_for_signal},
};
use jsonrpsee::{
    core::async_trait,
    http_client::{transport::HttpBackend, HttpClientBuilder},
    server::{RpcServiceBuilder, ServerBuilder},
};
use op_alloy_rpc_types_engine::{OpExecutionPayloadEnvelopeV3, OpPayloadAttributes};
use parking_lot::{Mutex, RwLock};
use reqwest::Url;
use reth_rpc_layer::{AuthClientLayer, AuthClientService, JwtSecret};
use tracing::{debug, error, info, Instrument, Level};

use crate::{cli::PortalArgs, middleware::ProxyService};

pub type HttpClient = jsonrpsee::http_client::HttpClient<AuthClientService<HttpBackend>>;

#[derive(Clone)]
struct Gateway {
    id: Url,
    client: HttpClient,
}

impl fmt::Debug for Gateway {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.id)
    }
}

#[derive(Clone)]
pub struct PortalServer {
    fallback_client: HttpClient,
    next_gateway_index: Arc<AtomicUsize>,
    next_gateway: Arc<Mutex<Gateway>>,
    gateway_clients: Arc<RwLock<Vec<Gateway>>>,
}

async fn refresh_gateway_clients(url: Url, gateway_jwt: JwtSecret, timeout: Duration) -> eyre::Result<Vec<Gateway>> {
    let response = reqwest::get(url).await?;
    let body = response.text().await?;
    let urls: Vec<Url> = serde_json::from_str(&body)?;

    let urls_d = urls.iter().map(|url| url.to_string()).collect::<Vec<String>>().join(",");
    debug!(urls = urls_d, "refreshed gateway clients");

    urls.into_iter().map(|url| create_gateway_client(url, gateway_jwt, timeout)).collect()
}

impl PortalServer {
    pub fn new(args: PortalArgs) -> eyre::Result<Self> {
        let gateway_jwt = args.gateway_jwt()?;
        let fallback_jwt = args.fallback_jwt()?;

        let fallback_client =
            create_auth_client(args.fallback_url, fallback_jwt, Duration::from_millis(args.fallback_timeout_ms))?;
        let gateway_client = create_gateway_client(
            args.gateway_url,
            gateway_jwt.clone(),
            Duration::from_millis(args.gateway_timeout_ms),
        )?;

        let gateway_clients = Arc::new(RwLock::new(vec![gateway_client.clone()]));

        let gateway_clients_c = gateway_clients.clone();

        if let Some(gateway_update_url) = args.gateway_update_url {
            tokio::spawn(async move {
                loop {
                    match refresh_gateway_clients(
                        gateway_update_url.clone(),
                        gateway_jwt,
                        Duration::from_millis(args.gateway_timeout_ms),
                    )
                    .await
                    {
                        Ok(clients) => {
                            info!(clients = clients.len(), "refreshed gateway clients");
                            *gateway_clients_c.write() = clients;
                        }
                        Err(err) => {
                            error!(%err, "failed to refresh gateway clients");
                        }
                    }

                    tokio::time::sleep(Duration::from_secs(30)).await;
                }
            });
        }

        let next_gateway = Arc::new(Mutex::new(gateway_client));
        let next_gateway_index = Arc::new(AtomicUsize::new(0));
        Ok(Self { fallback_client, gateway_clients, next_gateway, next_gateway_index })
    }

    pub async fn run(self, addr: SocketAddr) -> eyre::Result<()> {
        let fallback_client = self.fallback_client.clone();
        let rpc_middleware =
            RpcServiceBuilder::new().layer_fn(move |s| ProxyService::new(CAPABILITIES, s, fallback_client.clone()));

        let server = ServerBuilder::default().set_rpc_middleware(rpc_middleware).build(addr).await?;

        let mut module = EngineApiServer::into_rpc(self.clone());
        module.merge(MinimalEthApiServer::into_rpc(self)).expect("failed to merge modules");

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

    fn next_gateway(&self) -> Gateway {
        self.next_gateway.lock().clone()
    }

    fn refresh_next(&self) -> Gateway {
        let next_index = self.next_gateway_index.fetch_add(1, Ordering::Relaxed);
        let clients = self.gateway_clients.read();
        let mut lock = self.next_gateway.lock();
        *lock = clients[next_index % clients.len()].clone();
        lock.clone()
    }

    fn gateways(&self) -> Vec<Gateway> {
        self.gateway_clients.read().clone()
    }
}

/// This is a temporary API to broacast transactions to both gateway and fallback. In practice this should not be
/// receiving user facing calls so we need to find another way to do this
#[async_trait]
impl MinimalEthApiServer for PortalServer {
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

        let response = self.fallback_client.send_raw_transaction(bytes).await?;
        Ok(response)
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

        if payload_attributes.is_some() {
            // pick only one gateway for this block
            let gateway = self.refresh_next();
            let payload_attributes = payload_attributes.clone();
            tokio::spawn(
                async move {
                    match gateway.client.fork_choice_updated_v3(fork_choice_state, payload_attributes).await {
                        Ok(res) => {
                            if res.is_valid() {
                                debug!(?gateway, ?res, "gateway response");
                            } else {
                                error!(?gateway, ?res, "gateway response");
                            }
                        }
                        Err(err) => error!(?gateway, %err, "failed gateway"),
                    }
                }
                .in_current_span(),
            );
        } else {
            // send to all gateways
            for gateway in self.gateways() {
                let payload_attributes = payload_attributes.clone();
                tokio::spawn(
                    async move {
                        match gateway.client.fork_choice_updated_v3(fork_choice_state, payload_attributes).await {
                            Ok(res) => {
                                if res.is_valid() {
                                    debug!(?gateway, ?res, "gateway response");
                                } else {
                                    error!(?gateway, ?res, "gateway response");
                                }
                            }
                            Err(err) => error!(%err, "failed gateway"),
                        }
                    }
                    .in_current_span(),
                );
            }
        }

        let response = self.fallback_client.fork_choice_updated_v3(fork_choice_state, payload_attributes).await?;

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
                        Err(err) => error!(?gateway, %err, "failed gateway"),
                    }
                }
                .in_current_span(),
            );
        }

        let response = self.fallback_client.new_payload_v3(payload, versioned_hashes, parent_beacon_block_root).await?;
        Ok(response)
    }

    #[tracing::instrument(skip_all, err, ret(level = Level::DEBUG), fields(req_id = %uuid()))]
    async fn get_payload_v3(&self, payload_id: PayloadId) -> RpcResult<OpExecutionPayloadEnvelopeV3> {
        debug!(%payload_id, "new request");

        let fallback_fut = tokio::spawn({
            let client = self.fallback_client.clone();

            async move { client.get_payload_v3(payload_id).await }
        });

        let gateway_fut: tokio::task::JoinHandle<Result<OpExecutionPayloadEnvelopeV3, _>> = tokio::spawn(
            {
                // only get payload from previously picked gateway
                let gateway = self.next_gateway();
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
                        debug!(?gateway, ?gateway_payload, ?payload_status, "gateway response");
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

        let payload = gateway.or(fallback)?;

        Ok(payload)
    }
}

fn create_auth_client(url: Url, jwt: JwtSecret, timeout: Duration) -> eyre::Result<HttpClient> {
    let secret_layer = AuthClientLayer::new(jwt);
    let middleware = tower::ServiceBuilder::default().layer(secret_layer);

    let client = HttpClientBuilder::default().set_http_middleware(middleware).request_timeout(timeout).build(url)?;

    Ok(client)
}

fn create_gateway_client(url: Url, jwt: JwtSecret, timeout: Duration) -> eyre::Result<Gateway> {
    let client = create_auth_client(url.clone(), jwt, timeout)?;
    let gateway_client = Gateway { client, id: url };
    Ok(gateway_client)
}
