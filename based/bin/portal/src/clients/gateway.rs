use std::{
    fmt,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::SystemTime,
};

use alloy_eips::eip7685::RequestsOrHash;
use alloy_primitives::{Address, B256, Signature, U64};
use alloy_rpc_types::engine::{ExecutionPayloadV3, ForkchoiceState};
use arc_cell::{ArcCell, OptionalArcCell};
use bop_common::{
    api::{BasedAuthApiClient, ControlApiClient, EngineApiClient, OpMinerExtApiClient, RegistryApiClient},
    auth::gateway_auth_message,
    communication::Producer,
    debug_panic,
    metrics::{Gauge, Metric, MetricsUpdate},
    signing::ECDSASigner,
    time::{Duration, Instant},
    utils::uuid,
};
use eyre::eyre;
use indexmap::IndexMap;
use jsonrpsee::{
    core::{ClientError, middleware::layer::RpcLogger},
    http_client::{HttpClient, HttpClientBuilder, HttpRequest, RpcService, transport::HttpBackend},
};
use op_alloy_rpc_types_engine::{OpExecutionPayloadV4, OpPayloadAttributes};
use reqwest::{
    Url,
    header::{AUTHORIZATION, HeaderValue, InvalidHeaderValue},
};
use tokio::sync::RwLock;
use tower::{Layer, Service};
use tracing::{Instrument, debug, error, info, trace, warn};

use super::RpcClient;
use crate::{cli::PortalArgs, clients::create_client};

pub type GatewayClient = HttpClient<RpcLogger<RpcService<GatewayClientService<HttpBackend>>>>;

#[derive(Debug)]
pub struct GatewayClientAuthLayer {
    pub token: Arc<HeaderValue>,
}
impl GatewayClientAuthLayer {
    pub fn new(token: &str) -> Result<Self, InvalidHeaderValue> {
        let header = if token.starts_with("Bearer ") { token.to_owned() } else { format!("Bearer {token}") };
        let value = HeaderValue::from_str(&header)?;
        Ok(Self { token: Arc::new(value) })
    }
}

impl<S> Layer<S> for GatewayClientAuthLayer {
    type Service = GatewayClientService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        GatewayClientService { jwt: self.token.clone(), inner }
    }
}

#[derive(Debug, Clone)]
pub struct GatewayClientService<S> {
    jwt: Arc<HeaderValue>,
    inner: S,
}

impl<S, B> Service<HttpRequest<B>> for GatewayClientService<S>
where
    S: Service<HttpRequest<B>>,
{
    type Error = S::Error;
    type Future = S::Future;
    type Response = S::Response;

    fn poll_ready(&mut self, cx: &mut std::task::Context<'_>) -> std::task::Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: HttpRequest<B>) -> Self::Future {
        req.headers_mut().insert(AUTHORIZATION, Arc::unwrap_or_clone(self.jwt.clone()));
        self.inner.call(req)
    }
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;

    use jsonrpsee::http_client::HttpRequest;
    use reqwest::header::AUTHORIZATION;
    use tower::{Layer, ServiceExt, service_fn};

    use super::GatewayClientAuthLayer;

    #[tokio::test]
    async fn gateway_client_auth_layer_prefixes_bearer() {
        let layer = GatewayClientAuthLayer::new("token").expect("header should be valid");
        assert_eq!(layer.token.to_str().unwrap(), "Bearer token");

        let layer = GatewayClientAuthLayer::new("Bearer token").expect("header should be valid");
        assert_eq!(layer.token.to_str().unwrap(), "Bearer token");
    }

    #[tokio::test]
    async fn gateway_client_auth_layer_inserts_authorization_header() {
        let layer = GatewayClientAuthLayer::new("token").expect("header should be valid");

        // Simple service to retrieve the authorization header (which should have been added by the layer).
        let svc = service_fn(|req: HttpRequest<()>| async move {
            let auth = req.headers().get(AUTHORIZATION).cloned();
            Ok::<_, Infallible>(auth)
        });

        let svc = layer.layer(svc);
        let auth = svc
            .oneshot(HttpRequest::new(()))
            .await
            .expect("service call ok")
            .expect("authorization header should have been extracted");

        assert_eq!(auth.to_str().unwrap(), "Bearer token");
    }
}

#[derive(Clone)]
pub struct Gateway {
    pub url: Url,
    pub address: Arc<Address>,
    pub client: OptionalArcCell<GatewayClient>,
    pub ping: ArcCell<Duration>,
    pub active: Arc<AtomicBool>,
    pub registry_index: Arc<AtomicU64>,
    pub metrics: Producer<MetricsUpdate>,
}

impl Gateway {
    pub async fn health_check(&self) {
        let Some(client) = self.client.get() else {
            self.active.store(false, Ordering::Relaxed);
            return;
        };

        let ping_start = Instant::now();
        match ControlApiClient::heartbeat(&*client).await {
            Ok(_) => {
                let ping_duration = ping_start.elapsed();
                self.ping.set(ping_duration.into());
                self.active.store(true, Ordering::Relaxed);
                info!("successfully pinged gateway={} ping={:>9}", self.url, ping_duration.to_string());
                MetricsUpdate::send_ref(
                    uuid(),
                    Metric::SetGauge(Gauge::PortalGatewayPingLatencyMs(*self.address), ping_duration.as_millis()),
                    &self.metrics,
                );
            }
            Err(err) => {
                // TODO: specifically handle authentication error by removing client?
                // To also trigger re-authentication
                error!(%err, ?self, "failed to ping gateway");
                self.active.store(false, Ordering::Relaxed);
            }
        }
    }

    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::Relaxed)
    }

    /// Try to reauthenticate with the given gateway
    pub async fn authenticate(&self, timestamp: u64, signature: Signature, timeout: Duration) -> eyre::Result<()> {
        let client = self.client.take();

        // Invalidate the old client and request a new token.
        // Attempt to reuse the client if one exists
        let response = match client {
            None => {
                let temp_client = create_client(self.url.clone(), timeout)?;
                temp_client.authenticate_proposer(timestamp, signature).await
            }
            Some(old_client) => old_client.authenticate_proposer(timestamp, signature).await,
        }
        .map_err(|err| eyre!("gateway authentication RPC failed: {err}"))?;

        let auth_layer = GatewayClientAuthLayer::new(&response.token)?;
        let middleware = tower::ServiceBuilder::default().layer(auth_layer);

        let new_client = HttpClientBuilder::default()
            .max_request_size(u32::MAX)
            .max_response_size(u32::MAX)
            .set_http_middleware(middleware)
            .request_timeout(timeout.into())
            .build(self.url.clone())?;

        self.client.set(Some(new_client.into()));

        info!(url = %self.url, "authenticated gateway");

        Ok(())
    }

    pub fn client(&self) -> Option<Arc<GatewayClient>> {
        self.client.get()
    }
}

impl fmt::Debug for Gateway {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.url)
    }
}

impl Gateway {
    fn new(url: Url, address: Address, registry_index: usize, metrics: Producer<MetricsUpdate>) -> Self {
        Self {
            url,
            address: Arc::new(address),
            client: Default::default(),
            ping: Default::default(),
            active: Arc::new(AtomicBool::new(false)),
            registry_index: Arc::new(AtomicU64::new(registry_index as u64)),
            metrics,
        }
    }
}

pub type GatewayInstance = Arc<Gateway>;

pub struct GatewayManager {
    gateways: Arc<RwLock<IndexMap<Url, GatewayInstance>>>,
    pub registry_client: RpcClient,
    current_gateway: Arc<RwLock<Option<GatewayInstance>>>,
    pub metrics: Producer<MetricsUpdate>,

    // TODO: refactor out in dedicated auth structs
    authentication_signer: Arc<ECDSASigner>,

    gateway_timeout: Duration,
}

impl GatewayManager {
    pub fn new(
        registry_client: RpcClient,
        metrics: Producer<MetricsUpdate>,
        signer: Arc<ECDSASigner>,
        gateway_timeout: Duration,
    ) -> Self {
        Self {
            gateways: Arc::new(RwLock::new(IndexMap::new())),
            registry_client,
            current_gateway: Arc::new(RwLock::new(None)),
            metrics,
            authentication_signer: signer,
            gateway_timeout,
        }
    }

    pub fn new_from_args(args: &PortalArgs, signer: Arc<ECDSASigner>, metrics: Producer<MetricsUpdate>) -> Self {
        let registry_client_url = args.registry_url.clone();
        let timeout = Duration::from_millis(args.registry_timeout_ms);
        let registry_client = create_client(registry_client_url, timeout).unwrap();
        let gateway_timeout = Duration::from_millis(args.gateway_timeout_ms);
        Self::new(registry_client, metrics, signer, gateway_timeout)
    }

    pub async fn update_gateway_list(&self) -> eyre::Result<()> {
        let raw_gateways = self.registry_client.registered_gateways().await?;
        let mut gateways = self.gateways.write().await;

        gateways.retain(|url, _| raw_gateways.iter().any(|(u, _)| u == url));

        let expected_gateways = raw_gateways.len();

        for (index, (url, address)) in raw_gateways.into_iter().enumerate() {
            match gateways.get_mut(&url) {
                Some(gateway) => {
                    let gateway = Arc::make_mut(gateway);
                    gateway.address = Arc::new(address);
                    gateway.registry_index.store(index as u64, Ordering::Relaxed);
                }
                None => {
                    let gateway = Gateway::new(url.clone(), address, index, self.metrics);
                    gateways.insert(url, Arc::new(gateway));
                }
            }
        }

        let actual_gateways = gateways.len();
        if actual_gateways != expected_gateways {
            error!("Mismatch in number of gateways: expected {}, found {}", expected_gateways, actual_gateways);
            debug_panic!("Mismatch in number of gateways: expected {}, found {}", expected_gateways, actual_gateways);
        }

        gateways.sort_by(|_, v1, _, v2| {
            let index1 = v1.registry_index.load(Ordering::Relaxed);
            let index2 = v2.registry_index.load(Ordering::Relaxed);
            index1.cmp(&index2)
        });
        Ok(())
    }

    async fn get_next_available_gateway(&self, start_index: usize) -> Option<GatewayInstance> {
        let gateways = self.gateways.read().await;
        let len = gateways.len();
        for i in 0..len {
            let index = (start_index + i) % len;
            if let Some((_, gateway)) = gateways.get_index(index) {
                if gateway.is_active() {
                    return Some(Arc::clone(gateway));
                }
            }
        }
        None
    }

    // this method should be called at the block transition (end of block / start of new block)
    pub async fn decide_current_gateway(&self) -> Option<GatewayInstance> {
        let gateways = self.gateways.read().await;

        let gateway = match self.registry_client.current_gateway().await {
            Ok((_, url, _)) => match gateways.get_index_of(&url) {
                Some(index) => self.get_next_available_gateway(index).await,
                None => {
                    error!("Current registry gateway not found in local list: {}", url);
                    None
                }
            },
            Err(_) => {
                error!("Failed to fetch current gateway from registry");
                None
            }
        }?;

        Self::prepare_gateway(&self.authentication_signer, self.gateway_timeout, &gateway).await.ok()?;

        self.current_gateway.write().await.replace(gateway.clone());

        MetricsUpdate::send_ref(
            uuid(),
            Metric::SetGauge(Gauge::PortalCurrentGatewayRegistryAddress(*gateway.address), 1.0),
            &self.metrics,
        );

        Some(gateway)
    }

    async fn authenticate_gateway(
        signer: &ECDSASigner,
        timeout: Duration,
        gateway: GatewayInstance,
        valid_from: SystemTime,
    ) -> eyre::Result<()> {
        let timestamp = valid_from.duration_since(std::time::UNIX_EPOCH).expect("Time went backwards").as_secs();

        let message = gateway_auth_message(*gateway.address, timestamp);
        let signature =
            signer.sign_message(message).map_err(|err| eyre!("failed to sign gateway auth payload: {err}"))?;

        gateway.authenticate(timestamp, signature, timeout).await?;

        Ok(())
    }

    /// Attempts to authenticate with the gateway if it's unauthenticated
    async fn prepare_gateway(signer: &ECDSASigner, timeout: Duration, gateway: &GatewayInstance) -> eyre::Result<()> {
        if gateway.client().is_some() {
            return Ok(());
        }

        if let Err(err) = Self::authenticate_gateway(signer, timeout, gateway.clone(), SystemTime::now()).await {
            error!(%err, url = %gateway.url, "failed to authenticate gateway");
            return Err(err);
        }

        Ok(())
    }

    pub async fn health_check(&self) {
        let timeout = self.gateway_timeout;
        let gateways = self.gateways.read().await;
        for gateway in gateways.values().collect::<Vec<_>>() {
            let gateway = gateway.clone();
            let signer = self.authentication_signer.clone();

            tokio::spawn(async move {
                gateway.health_check().await;
                // TODO: only if health check fails due to auth
                _ = Self::prepare_gateway(&signer, timeout, &gateway).await;
            });
        }
    }

    pub async fn current_gateway(&self) -> Option<GatewayInstance> {
        self.current_gateway.read().await.as_ref().cloned()
    }

    async fn _send_fcu(
        fork_choice_state: ForkchoiceState,
        payload_attributes: Option<OpPayloadAttributes>,
        gateway: GatewayInstance,
    ) {
        let Some(client) = gateway.client() else {
            warn!(url = %gateway.url, "cannot send fork choice update to unauthenticated gateway");
            return;
        };

        match client.fork_choice_updated_v3(fork_choice_state, payload_attributes).await {
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

    pub async fn send_fcu(&self, fork_choice_state: ForkchoiceState, payload_attributes: Option<OpPayloadAttributes>) {
        match self.current_gateway().await {
            Some(gateway) => {
                if gateway.is_active() {
                    tokio::spawn(Self::_send_fcu(fork_choice_state, payload_attributes, gateway));
                } else {
                    error!("Current gateway is not active, cannot send fork choice update");
                }
            }
            None => {
                for gateway in self.gateways.read().await.values() {
                    let payload_attributes = payload_attributes.clone();
                    tokio::spawn(Self::_send_fcu(fork_choice_state, payload_attributes, gateway.clone()));
                }
            }
        }
    }

    pub async fn broadcast_new_payload_v3(
        &self,
        payload: ExecutionPayloadV3,
        versioned_hashes: Vec<B256>,
        parent_beacon_block_root: B256,
    ) {
        for gateway in self.gateways.read().await.values() {
            let gateway = gateway.clone();
            let payload = payload.clone();
            let versioned_hashes = versioned_hashes.clone();
            tokio::spawn(
                async move {
                    let Some(client) = gateway.client() else {
                        warn!(url = %gateway.url, "skipping newPayloadV3 broadcast to unauthenticated gateway");
                        return;
                    };
                    match client.new_payload_v3(payload, versioned_hashes, parent_beacon_block_root).await {
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
    }

    pub async fn broadcast_new_payload_v4(
        &self,
        payload: OpExecutionPayloadV4,
        versioned_hashes: Vec<B256>,
        parent_beacon_block_root: B256,
        requests: RequestsOrHash,
    ) {
        for gateway in self.gateways.read().await.values() {
            let gateway = gateway.clone();
            let payload = payload.clone();
            let requests = requests.clone();
            let versioned_hashes = versioned_hashes.clone();
            tokio::spawn(
                async move {
                    let Some(client) = gateway.client() else {
                        warn!(url = %gateway.url, "skipping newPayloadV4 broadcast to unauthenticated gateway");
                        return;
                    };
                    match client.new_payload_v4(payload, versioned_hashes, parent_beacon_block_root, requests).await {
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
    }

    pub async fn broadcast_set_max_da_size(&self, max_tx_size: U64, max_block_size: U64) {
        for gateway in self.gateways.read().await.values() {
            let gateway = gateway.clone();
            tokio::spawn(
                async move {
                    let Some(client) = gateway.client() else {
                        warn!(url = %gateway.url, "skipping miner_setMaxDASize for unauthenticated gateway");
                        return;
                    };
                    if let Err(err) = client.set_max_da_size(max_tx_size, max_block_size).await {
                        error!(?gateway, %err, "failed to forward miner_setMaxDASize");
                    }
                }
                .in_current_span(),
            );
        }
    }
}
