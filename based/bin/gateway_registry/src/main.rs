use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use alloy_primitives::{Address, B256, U256};
use bop_common::{
    api::{EthApiClient, RegistryApiServer},
    communication::messages::{RpcError, RpcResult},
    config::LoggingConfig,
    utils::{init_tracing, wait_for_signal},
};
use clap::Parser;
use jsonrpsee::{core::async_trait, http_client::HttpClientBuilder, server::ServerBuilder};
use parking_lot::RwLock;
use reqwest::Url;
use thiserror::Error;
use tracing::{error, info, level_filters::LevelFilter, Level};

pub type RpcClient = jsonrpsee::http_client::HttpClient;
#[derive(Parser, Debug, Clone)]
#[command(version, about, name = "gateway-registry")]
pub struct RegistryArgs {
    /// The host to run the registry on
    #[arg(long = "registry.host", default_value_t = Ipv4Addr::UNSPECIFIED)]
    pub registry_host: Ipv4Addr,

    /// The port to run the registry on
    #[arg(long = "registry.port", default_value_t = 8081)]
    pub registry_port: u16,

    /// TEMP: The path to store the registry to
    #[arg(long = "registry.path")]
    pub registry_path: std::path::PathBuf,

    /// The url of the portal
    #[arg(long = "eth_client.url")]
    pub eth_client_url: Url,

    /// Timeout when trying to contact the portal
    #[arg(long = "eth_client.timeout_ms", default_value_t = 1_000)]
    pub eth_client_timeout: u64,

    /// Enable debug logging
    #[arg(long)]
    pub debug: bool,

    /// Enable trace logging
    #[arg(long)]
    pub trace: bool,

    /// Each gateway gets selected for this number of consecutive L2 blocks
    #[arg(long = "gateway.update_interval_blocks", default_value_t = 30)]
    pub gateway_update_blocks: u64,
}

impl From<&RegistryArgs> for LoggingConfig {
    fn from(args: &RegistryArgs) -> Self {
        Self {
            level: args
                .trace
                .then_some(LevelFilter::TRACE)
                .or(args.debug.then_some(LevelFilter::DEBUG))
                .unwrap_or(LevelFilter::INFO),
            enable_file_logging: false,
            prefix: None,
            max_files: 100,
            path: PathBuf::from("/tmp"),
            filters: None,
        }
    }
}

#[derive(Debug, Error)]
enum RegistryError {
    #[error("File system error {0}")]
    FileSystem(#[from] std::io::Error),
    #[error("parsing error {0}")]
    Parse(#[from] serde_json::Error),
}
type Result<T> = std::result::Result<T, RegistryError>;

fn refresh_gateway_clients(path: impl AsRef<Path>) -> Result<Vec<(Url, Address, B256)>> {
    Ok(serde_json::from_reader(std::fs::File::open(path.as_ref())?)?)
}

#[derive(Clone)]
pub struct RegistryServer {
    eth_client: RpcClient,
    // url, address, jwt secret
    gateway_clients: Arc<RwLock<Vec<(Url, Address, B256)>>>,
    gateway_update_blocks: u64,
}

impl RegistryServer {
    pub fn new(args: RegistryArgs) -> eyre::Result<Self> {
        let portal_eth_client = create_client(args.eth_client_url, Duration::from_millis(args.eth_client_timeout))?;

        let gateway_clients = Arc::new(RwLock::new(refresh_gateway_clients(&args.registry_path).unwrap_or_default()));
        let gateway_clients_cloned = gateway_clients.clone();
        tokio::spawn(async move {
            loop {
                match refresh_gateway_clients(&args.registry_path) {
                    Ok(clients) => {
                        info!(clients = clients.len(), "refreshed gateway clients");
                        *gateway_clients_cloned.write() = clients;
                    }
                    Err(err) => {
                        error!(%err, "failed to refresh gateway clients");
                    }
                }

                tokio::time::sleep(Duration::from_secs(30)).await;
            }
        });
        while gateway_clients.read().is_empty() {
            info!("Waiting until at least one gateway becomes available");
            std::thread::sleep(Duration::from_millis(200));
        }

        Ok(Self { eth_client: portal_eth_client, gateway_clients, gateway_update_blocks: args.gateway_update_blocks })
    }

    pub async fn run(self, addr: SocketAddr) -> eyre::Result<()> {
        let server = ServerBuilder::default()
            .max_request_body_size(u32::MAX)
            .max_response_body_size(u32::MAX)
            .build(addr)
            .await?;

        let module = RegistryApiServer::into_rpc(self.clone());
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
}

/// This is a temporary API to broacast transactions to both gateway and fallback. In practice this should not be
/// receiving user facing calls so we need to find another way to do this
#[async_trait]
impl RegistryApiServer for RegistryServer {
    #[tracing::instrument(skip_all, err, ret(level = Level::DEBUG))]
    async fn get_future_gateway(&self, n_blocks_into_the_future: u64) -> RpcResult<(u64, Url, Address, B256)> {
        info!(n_blocks_into_the_future, "serving future gateway");
        let curblock = self.eth_client.block_number().await?;
        let gateways = self.gateway_clients.read();
        let n_gateways = gateways.len();
        let target_block = u64::try_from(curblock + U256::from_limbs([1, 0, 0, 0])).map_err(|_| RpcError::Internal)? +
            n_blocks_into_the_future;
        let id = (target_block / self.gateway_update_blocks) as usize;
        let (url, address, jwt_in_b256) = gateways[id % n_gateways].clone();
        Ok((target_block, url, address, jwt_in_b256))
    }

    async fn registered_gateways(&self) -> RpcResult<Vec<(Url, Address, B256)>> {
        Ok(self.gateway_clients.read().clone())
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

#[tokio::main]
async fn main() -> eyre::Result<()> {
    let args = RegistryArgs::parse();
    let _guard = init_tracing((&args).into());

    let addr = SocketAddr::new(IpAddr::V4(args.registry_host), args.registry_port);
    let server = RegistryServer::new(args.clone())?;

    info!(%addr,  eth_client_url = %args.eth_client_url, "starting Based Registry");
    server.run(addr).await
}
