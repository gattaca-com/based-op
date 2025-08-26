use std::{
    net::SocketAddr,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicI64, Ordering},
    },
};

use bop_common::{
    communication::Producer,
    metrics::{Counter, Gauge, Metric, MetricsUpdate, metrics_queue},
    time::Duration,
    utils::{uuid, wait_for_signal},
};
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

#[derive(Debug, Clone)]
pub struct FlowCounter {
    pub total: Arc<AtomicI64>,

    pub success: Arc<AtomicI64>,
    pub failed: Arc<AtomicI64>,

    pub failed_invalid_params: Arc<AtomicI64>,
    pub failed_method_not_found: Arc<AtomicI64>,
    pub failed_no_clients: Arc<AtomicI64>,
    pub failed_all_clients: Arc<AtomicI64>,
}

impl Default for FlowCounter {
    fn default() -> Self {
        Self {
            total: Arc::new(AtomicI64::new(0)),
            success: Arc::new(AtomicI64::new(0)),
            failed: Arc::new(AtomicI64::new(0)),
            failed_invalid_params: Arc::new(AtomicI64::new(0)),
            failed_method_not_found: Arc::new(AtomicI64::new(0)),
            failed_no_clients: Arc::new(AtomicI64::new(0)),
            failed_all_clients: Arc::new(AtomicI64::new(0)),
        }
    }
}

impl FlowCounter {
    pub fn increment_total(&self) {
        self.total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn increment_success(&self) {
        self.success.fetch_add(1, Ordering::Relaxed);
    }

    pub fn increment_failed_invalid_params(&self) {
        self.failed_invalid_params.fetch_add(1, Ordering::Relaxed);
        self.failed.fetch_add(1, Ordering::Relaxed);
    }

    pub fn increment_failed_method_not_found(&self) {
        self.failed_method_not_found.fetch_add(1, Ordering::Relaxed);
        self.failed.fetch_add(1, Ordering::Relaxed);
    }

    pub fn increment_failed_no_clients(&self) {
        self.failed_no_clients.fetch_add(1, Ordering::Relaxed);
        self.failed.fetch_add(1, Ordering::Relaxed);
    }

    pub fn increment_failed_all_clients(&self) {
        self.failed_all_clients.fetch_add(1, Ordering::Relaxed);
        self.failed.fetch_add(1, Ordering::Relaxed);
    }

    pub fn print_summary_and_reset(&self) {
        info!(
            total = self.total.load(Ordering::Relaxed),
            success = self.success.load(Ordering::Relaxed),
            failed = self.failed.load(Ordering::Relaxed),
            failed_invalid_params = self.failed_invalid_params.load(Ordering::Relaxed),
            failed_method_not_found = self.failed_method_not_found.load(Ordering::Relaxed),
            failed_no_clients = self.failed_no_clients.load(Ordering::Relaxed),
            failed_all_clients = self.failed_all_clients.load(Ordering::Relaxed),
            "Flow statistics"
        );

        // Reset counters
        self.total.store(0, Ordering::Relaxed);
        self.success.store(0, Ordering::Relaxed);
        self.failed.store(0, Ordering::Relaxed);
        self.failed_invalid_params.store(0, Ordering::Relaxed);
        self.failed_method_not_found.store(0, Ordering::Relaxed);
        self.failed_no_clients.store(0, Ordering::Relaxed);
        self.failed_all_clients.store(0, Ordering::Relaxed);
    }

    pub fn send_metrics(&self, metrics: &Producer<MetricsUpdate>) {
        let id = uuid();
        MetricsUpdate::send_ref(
            id,
            Metric::IncrementCounter(Counter::TxProxyTotalRequests, self.total.load(Ordering::Relaxed) as u64),
            metrics,
        );
        MetricsUpdate::send_ref(
            id,
            Metric::IncrementCounter(Counter::TxProxyTotalRequests, self.total.load(Ordering::Relaxed) as u64),
            metrics,
        );
        MetricsUpdate::send_ref(
            id,
            Metric::IncrementCounter(Counter::TxProxyFailedRequests, self.failed.load(Ordering::Relaxed) as u64),
            metrics,
        );
        MetricsUpdate::send_ref(
            id,
            Metric::IncrementCounter(
                Counter::TxProxyFailedRequestsInvalidParams,
                self.failed_invalid_params.load(Ordering::Relaxed) as u64,
            ),
            metrics,
        );
        MetricsUpdate::send_ref(
            id,
            Metric::IncrementCounter(
                Counter::TxProxyFailedRequestsMethodNotFound,
                self.failed_method_not_found.load(Ordering::Relaxed) as u64,
            ),
            metrics,
        );
        MetricsUpdate::send_ref(
            id,
            Metric::IncrementCounter(
                Counter::TxProxyFailedRequestsNoClients,
                self.failed_no_clients.load(Ordering::Relaxed) as u64,
            ),
            metrics,
        );
        MetricsUpdate::send_ref(
            id,
            Metric::IncrementCounter(
                Counter::TxProxyFailedRequestsAllClients,
                self.failed_all_clients.load(Ordering::Relaxed) as u64,
            ),
            metrics,
        );
    }
}

#[derive(Clone)]
pub struct TxProxyServer {
    forward_to: Arc<RwLock<Vec<HttpClient>>>,
    flow_counter: Arc<FlowCounter>,
    args: Arc<TxProxyArgs>,
    metrics: Producer<MetricsUpdate>,
}

impl TxProxyServer {
    pub async fn new(args: TxProxyArgs) -> eyre::Result<Self> {
        let temp = Self {
            forward_to: Arc::new(RwLock::new(vec![])),
            args: Arc::new(args),
            flow_counter: Arc::new(FlowCounter::default()),
            metrics: metrics_queue().into(),
        };

        Ok(temp)
    }

    pub async fn run(self, addr: SocketAddr) -> eyre::Result<()> {
        let cors = CorsLayer::new().allow_origin(Any).allow_methods(Any).allow_headers(Any);

        let multiplex_service = MultiplexingService::new(Arc::clone(&self.forward_to), Arc::clone(&self.flow_counter));
        let rpc_service = RpcServiceBuilder::new().layer_fn(move |_inner| multiplex_service.clone());

        let http_middleware = ServiceBuilder::new().layer(cors);

        let server = ServerBuilder::default()
            .max_request_body_size(u32::MAX)
            .max_response_body_size(u32::MAX)
            .set_rpc_middleware(rpc_service)
            .set_http_middleware(http_middleware)
            .build(addr)
            .await?;

        let metrics = self.metrics;
        let flow_counter = Arc::clone(&self.flow_counter);
        let flow_counter_info_task = tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(60).into()).await;
                flow_counter.send_metrics(&metrics);
                flow_counter.print_summary_and_reset();
            }
        });

        let url_list_refresher_task = tokio::spawn(async move {
            loop {
                let tx_receivers = refresh_tx_receivers(&self.args.tx_receivers_path).unwrap();
                let clients = tx_receivers
                    .iter()
                    .map(|url| create_client(url.clone(), Duration::from_millis(1000)).unwrap())
                    .collect();
                self.set_forwarding_clients(clients);
                info!(clients = tx_receivers.len(), "refreshed forwarding clients");
                MetricsUpdate::send_ref(
                    uuid(),
                    Metric::SetGauge(Gauge::TxProxyForwardingClients, tx_receivers.len() as f64),
                    &metrics,
                );
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

            _ = flow_counter_info_task => {
                error!("Flow counter info task stopped unexpectedly");
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
