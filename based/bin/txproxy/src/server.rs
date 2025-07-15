use std::{net::SocketAddr, path::Path, sync::Arc};

use bop_common::{time::Duration, utils::wait_for_signal};
use jsonrpsee::{
    Methods,
    http_client::{HttpClient, HttpClientBuilder},
    server::{RpcServiceBuilder, ServerBuilder},
};
use parking_lot::RwLock;
use reqwest::Url;
use thiserror::Error;
use tower::ServiceBuilder;
use tower_http::cors::{Any, CorsLayer};
use tracing::{error, info};

use crate::{cli::TxProxyArgs, middleware::MultiplexingService};

#[derive(Clone)]
pub struct TxProxyServer {
    forward_to: Arc<RwLock<Vec<HttpClient>>>,
    args: Arc<TxProxyArgs>,
}

impl TxProxyServer {
    pub async fn new(args: TxProxyArgs) -> eyre::Result<Self> {
        let temp = Self { forward_to: Arc::new(RwLock::new(vec![])), args: Arc::new(args) };

        Ok(temp)
    }

    pub async fn run(self, addr: SocketAddr) -> eyre::Result<()> {
        let cors = CorsLayer::new().allow_origin(Any).allow_methods(Any).allow_headers(Any);

        let multiplex_service = MultiplexingService::new(Arc::clone(&self.forward_to));
        let rpc_service = RpcServiceBuilder::new().layer_fn(move |_inner| multiplex_service.clone());

        let http_middleware = ServiceBuilder::new().layer(cors);

        let server = ServerBuilder::default()
            .max_request_body_size(u32::MAX)
            .max_response_body_size(u32::MAX)
            .set_rpc_middleware(rpc_service)
            .set_http_middleware(http_middleware)
            .build(addr)
            .await?;

        let url_list_refresher_task = tokio::spawn(async move {
            loop {
                let tx_receivers = refresh_tx_receivers(&self.args.tx_receivers_path).unwrap();
                let clients = tx_receivers
                    .iter()
                    .map(|url| create_client(url.clone(), Duration::from_millis(1000)).unwrap())
                    .collect();
                self.set_forwarding_clients(clients);
                info!(clients = tx_receivers.len(), "refreshed forwarding clients");
                tokio::time::sleep(Duration::from_secs(5).into()).await;
            }
        });

        let server_handle = server.start(Methods::new());

        tokio::select! {
            _ = server_handle.stopped() => {
                error!("server stopped");
            }

            _ = wait_for_signal() => {
                info!("received signal, shutting down");
            }

            _ = url_list_refresher_task => {
                error!("URL list refresher task stopped unexpectedly");
            }
        }

        Ok(())
    }

    pub fn set_forwarding_clients(&self, clients: Vec<HttpClient>) {
        let mut forwarding_clients = self.forward_to.write();
        *forwarding_clients = clients;
    }
}

fn create_client(url: Url, timeout: Duration) -> eyre::Result<HttpClient> {
    let client = HttpClientBuilder::default()
        .max_request_size(u32::MAX)
        .max_response_size(u32::MAX)
        .request_timeout(timeout.into())
        .build(url)?;
    Ok(client)
}

#[derive(Debug, Error)]
enum ProxyError {
    #[error("File system error {0}")]
    FileSystem(#[from] std::io::Error),
    #[error("parsing error {0}")]
    Parse(#[from] serde_json::Error),
}

type Result<T> = std::result::Result<T, ProxyError>;

fn refresh_tx_receivers(path: impl AsRef<Path>) -> Result<Vec<Url>> {
    Ok(serde_json::from_reader(std::fs::File::open(path.as_ref())?)?)
}
