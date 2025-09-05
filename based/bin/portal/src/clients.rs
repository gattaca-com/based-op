use std::{
    fmt,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use alloy_eips::eip7685::RequestsOrHash;
use alloy_primitives::{Address, B256, U64, hex};
use alloy_rpc_types::engine::{ExecutionPayloadV3, ForkchoiceState};
use bop_common::{
    api::{ControlApiClient, EngineApiClient, OpMinerExtApiClient, RegistryApiClient},
    debug_panic,
    time::{Duration, Instant},
};
use indexmap::IndexMap;
use jsonrpsee::{
    core::ClientError,
    http_client::{HttpClientBuilder, transport::HttpBackend},
};
use op_alloy_rpc_types_engine::{OpExecutionPayloadV4, OpPayloadAttributes};
use reqwest::Url;
use reth_rpc_layer::{AuthClientLayer, AuthClientService, JwtSecret};
use tokio::sync::RwLock;
use tracing::{Instrument, debug, error, info, trace};

use crate::cli::PortalArgs;

pub type RpcClient = jsonrpsee::http_client::HttpClient;
pub type AuthRpcClient = jsonrpsee::http_client::HttpClient<AuthClientService<HttpBackend>>;

#[derive(Clone)]
pub struct Gateway {
    pub url: Url,
    pub jwt: JwtSecret,
    pub address: Address,
    pub client: AuthRpcClient,
    pub ping: Arc<RwLock<Duration>>,
    pub active: Arc<AtomicBool>,
    pub registry_index: Arc<AtomicU64>,
}

impl Gateway {
    pub async fn health_check(&self) {
        let ping_start = Instant::now();
        match ControlApiClient::heartbeat(&self.client).await {
            Ok(_) => {
                let ping_duration = ping_start.elapsed();
                *self.ping.write().await = ping_duration;
                self.active.store(true, Ordering::Relaxed);
                info!("successfully pinged gateway={} ping={:>9}", self.url, ping_duration.to_string());
            }
            Err(err) => {
                error!(%err, ?self, "failed to ping gateway");
                self.active.store(false, Ordering::Relaxed);
            }
        }
    }

    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::Relaxed)
    }
}

impl fmt::Debug for Gateway {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.url)
    }
}

impl Gateway {
    fn new(url: Url, client: AuthRpcClient, jwt: JwtSecret, address: Address, registry_index: usize) -> Self {
        Self {
            url,
            jwt,
            address,
            client,
            ping: Arc::new(RwLock::new(Duration::from_millis(0))),
            active: Arc::new(AtomicBool::new(false)),
            registry_index: Arc::new(AtomicU64::new(registry_index as u64)),
        }
    }
}

pub type GatewayInstance = Arc<Gateway>;

pub struct GatewayManager {
    gateways: Arc<RwLock<IndexMap<Url, GatewayInstance>>>,
    pub registry_client: RpcClient,
    current_gateway: Arc<RwLock<Option<GatewayInstance>>>,
}

impl GatewayManager {
    pub fn new(registry_client: RpcClient) -> Self {
        Self {
            gateways: Arc::new(RwLock::new(IndexMap::new())),
            registry_client,
            current_gateway: Arc::new(RwLock::new(None)),
        }
    }

    pub fn new_from_args(args: &PortalArgs) -> Self {
        let registry_client_url = args.registry_url.clone();
        let timeout = Duration::from_millis(args.fallback_timeout_ms);
        let registry_client = create_client(registry_client_url, timeout).unwrap();
        Self::new(registry_client)
    }

    pub async fn update_gateway_list(&self) -> eyre::Result<()> {
        let raw_gateways = self.registry_client.registered_gateways().await?;
        let mut gateways = self.gateways.write().await;

        let timeout = Duration::from_millis(1000);

        for (index, (url, address, jwt_as_b256)) in raw_gateways.iter().enumerate() {
            let jwt_as_str = hex::encode(jwt_as_b256);
            let jwt = JwtSecret::from_hex(&jwt_as_str).map_err(|_| eyre::eyre!("Invalid JWT secret"))?;
            match gateways.get_mut(url) {
                Some(gateway) => {
                    let gateway = Arc::make_mut(gateway);
                    gateway.jwt = jwt;
                    gateway.address = *address;
                    gateway.registry_index.store(index as u64, Ordering::Relaxed);
                }
                None => {
                    let client = create_auth_client(url.clone(), jwt, timeout)?;
                    let gateway = Gateway::new(url.clone(), client, jwt, *address, index);
                    gateways.insert(url.clone(), Arc::new(gateway));
                }
            }
        }

        gateways.retain(|url, _| raw_gateways.iter().any(|(u, _, _)| u == url));

        if gateways.len() != raw_gateways.len() {
            error!("Mismatch in number of gateways: expected {}, found {}", raw_gateways.len(), gateways.len());
            debug_panic!("Mismatch in number of gateways: expected {}, found {}", raw_gateways.len(), gateways.len());
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
        match self.registry_client.current_gateway().await {
            Ok((_, current_registry_gateway_url, _, _)) => match gateways.get_index_of(&current_registry_gateway_url) {
                Some(index) => {
                    let result = self.get_next_available_gateway(index).await;
                    if let Some(gateway) = &result {
                        self.current_gateway.write().await.replace(Arc::clone(gateway));
                    }
                    result
                }
                None => {
                    error!("Current registry gateway not found in local list: {}", current_registry_gateway_url);
                    None
                }
            },
            Err(_) => {
                error!("Failed to fetch current gateway from registry");
                None
            }
        }
    }

    pub async fn health_check(&self) {
        let gateways = self.gateways.read().await;
        for gateway in gateways.values().collect::<Vec<_>>() {
            let gateway = gateway.clone();
            tokio::spawn(async move {
                gateway.health_check().await;
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
                    match gateway
                        .client
                        .new_payload_v4(payload, versioned_hashes, parent_beacon_block_root, requests)
                        .await
                    {
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
                    if let Err(err) = gateway.client.set_max_da_size(max_tx_size, max_block_size).await {
                        error!(?gateway, %err, "failed to forward miner_setMaxDASize");
                    }
                }
                .in_current_span(),
            );
        }
    }
}

pub fn create_client(url: Url, timeout: Duration) -> eyre::Result<RpcClient> {
    let client = HttpClientBuilder::default()
        .max_request_size(u32::MAX)
        .max_response_size(u32::MAX)
        .request_timeout(timeout.into())
        .build(url)?;
    Ok(client)
}

pub fn create_auth_client(url: Url, jwt: JwtSecret, timeout: Duration) -> eyre::Result<AuthRpcClient> {
    let secret_layer = AuthClientLayer::new(jwt);
    let middleware = tower::ServiceBuilder::default().layer(secret_layer);

    let client = HttpClientBuilder::default()
        .max_request_size(u32::MAX)
        .max_response_size(u32::MAX)
        .set_http_middleware(middleware)
        .request_timeout(timeout.into())
        .build(url)?;

    Ok(client)
}
