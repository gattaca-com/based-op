use std::{net::SocketAddr, sync::Arc};

use alloy_primitives::{Bytes, B256};
use alloy_rpc_types::engine::JwtSecret;
use bop_common::{
    api::{EngineApiServer, MinimalEthApiServer},
    communication::{
        messages::{EngineApi, RpcResult},
        Sender, Spine,
    },
    config::GatewayArgs,
    db::DatabaseRead,
    time::Duration,
    transaction::Transaction,
};
use jsonrpsee::{core::async_trait, server::ServerBuilder};
use reth_rpc_layer::{AuthLayer, JwtAuthValidator};
use tokio::runtime::Runtime;
use tracing::{error, info, trace, Level};

mod engine;
pub mod gossiper;

pub fn start_rpc<Db: DatabaseRead>(config: &GatewayArgs, spine: &Spine<Db>, rt: &Runtime) {
    let addr = SocketAddr::new(config.rpc_host.into(), config.rpc_port);
    let server = RpcServer::new(spine, config.rpc_jwt);
    rt.spawn(server.run(addr));
}

// TODO: jwt auth
// TODO: timing
#[derive(Debug, Clone)]
struct RpcServer {
    new_order_tx: Sender<Arc<Transaction>>,
    engine_timeout: Duration,
    engine_rpc_tx: Sender<EngineApi>,
    jwt: JwtSecret,
}

impl RpcServer {
    pub fn new<Db>(spine: &Spine<Db>, jwt: JwtSecret) -> Self {
        Self { new_order_tx: spine.into(), engine_rpc_tx: spine.into(), engine_timeout: Duration::from_secs(1), jwt }
    }

    #[tracing::instrument(skip_all, name = "rpc")]
    pub async fn run(self, addr: SocketAddr) {
        info!(%addr, "starting RPC server");
        let validator = JwtAuthValidator::new(self.jwt);
        let auth_layer = AuthLayer::new(validator);
        let service_builder = tower::ServiceBuilder::new()
            // Proxy `GET /health` requests to internal `system_health` method.
            .layer(auth_layer)
            .timeout(std::time::Duration::from_secs(2));

        let server = ServerBuilder::default()
            .max_request_body_size(u32::MAX)
            .max_response_body_size(u32::MAX)
            .set_http_middleware(service_builder)
            .build(addr)
            .await
            .expect("failed to create eth RPC server");
        let mut module = MinimalEthApiServer::into_rpc(self.clone());
        module.merge(EngineApiServer::into_rpc(self)).expect("failed to merge modules");

        let server_handle = server.start(module);
        //TODO: Handle other communcation from sequencer ?
        //      Idea: we have this part do rpc requests, using the rpc->sequencer channel,
        //      but we make it part of another sync actor that uses the connections and gathers
        //      state etc in a spinloop that the rpc runtime can use to serve requests with?
        server_handle.stopped().await;

        error!("server stopped");
    }
}

/// Note: this is a temporary RPC implementation that only serves the lastest state from the sequencer.
/// It doesn't adhere to the specific block number or hash requests.
/// This will ultimately be replaced by the RPC server in the EL when the full Frag handling is implemented.
#[async_trait]
impl MinimalEthApiServer for RpcServer {
    #[tracing::instrument(skip_all, err, ret(level = Level::TRACE))]
    async fn send_raw_transaction(&self, bytes: Bytes) -> RpcResult<B256> {
        trace!(?bytes, "new request");

        let tx = Arc::new(Transaction::decode(bytes)?);
        let hash = tx.tx_hash();
        let _ = self.new_order_tx.send(tx.into());

        Ok(hash)
    }
}
