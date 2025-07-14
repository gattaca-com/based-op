use std::{
    fmt,
    net::{IpAddr, SocketAddr},
    str::FromStr,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use alloy_eips::eip7685::RequestsOrHash;
use alloy_primitives::{Address, B256, Bytes, U256, hex};
use alloy_rpc_types::{
    BlockId, BlockNumberOrTag,
    engine::{ExecutionPayloadV3, ForkchoiceState, ForkchoiceUpdated, PayloadId, PayloadStatus},
};
use bop_common::{
    api::{
        ControlApiClient, EngineApiClient, EngineApiServer, EthApiClient, EthApiServer, OpGethAdminApiClient,
        OpNodeApiClient, OpNodeP2PApiClient, OpRpcBlock, PORTAL_CAPABILITIES, PortalApiServer, RegistryApiClient,
        RegistryApiServer,
    },
    communication::messages::{RpcError, RpcResult},
    debug_panic,
    time::{Duration, Instant},
    utils::{uuid, wait_for_signal},
};
use jsonrpsee::{
    Methods,
    core::{ClientError, async_trait},
    http_client::{HttpClient, HttpClientBuilder, transport::HttpBackend},
    server::{RpcServiceBuilder, ServerBuilder},
};
use op_alloy_rpc_types::OpTransactionReceipt;
use op_alloy_rpc_types_engine::{OpExecutionPayloadEnvelopeV4, OpExecutionPayloadV4, OpPayloadAttributes};
use parking_lot::RwLock;
use reqwest::Url;
use reqwest::get;
use reth_rpc_layer::{AuthClientLayer, AuthClientService, JwtSecret};
use tokio::sync::Mutex;
use tower::ServiceBuilder;
use tower_http::cors::{Any, CorsLayer};
use tracing::{Instrument, Level, debug, error, info, trace};

use crate::{cli::TxProxyArgs, middleware::MultiplexingService};

pub type RpcClient = jsonrpsee::http_client::HttpClient;
pub type AuthRpcClient = jsonrpsee::http_client::HttpClient<AuthClientService<HttpBackend>>;

#[derive(Clone)]
pub struct TxReceiver {
    pub client: HttpClient,
}

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

        let server_handle = server.start(Methods::new());

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

    pub fn add_forwarding_client(&self, client: HttpClient) {
        let mut forwarding_clients = self.forward_to.write();
        forwarding_clients.push(client);
    }
}
