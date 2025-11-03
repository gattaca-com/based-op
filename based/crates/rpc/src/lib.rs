use std::{net::SocketAddr, sync::Arc};

use alloy_primitives::{B256, Bytes, U64};
use alloy_rpc_types::engine::JwtSecret;
use axum::{Router, routing::get};
use bop_common::{
    api::{ControlApiServer, EngineApiServer, MinimalEthApiServer, OpMinerExtApiServer},
    communication::{
        Producer, Sender, Spine,
        messages::{EngineApi, RpcResult},
    },
    config::GatewayArgs,
    db::DatabaseRead,
    fabric::FabricGatewayApiServer,
    p2p::SignedVersionedMessage,
    telemetry::{TelemetryUpdate, telemetry_queue},
    time::Duration,
    transaction::Transaction,
};
use jsonrpsee::{
    core::async_trait,
    server::{ServerBuilder, ServerConfigBuilder},
};
use reth_optimism_payload_builder::config::OpDAConfig;
use reth_rpc_layer::{AuthLayer, JwtAuthValidator};
use tokio::{net::TcpListener, runtime::Runtime};
use tracing::{Level, error, info, trace};

use crate::state_stream::{StreamState, state_stream};

mod engine;
mod fabric;
pub mod gossiper;
mod state_stream;

const STATE_STREAM_PATH: &str = "/state_stream";

pub fn start_rpc<Db: DatabaseRead>(
    config: &GatewayArgs,
    spine: &Spine<Db>,
    rt: &Runtime,
    rx_spawner: tokio::sync::broadcast::Sender<SignedVersionedMessage>,
    da_config: OpDAConfig,
) {
    let addr_auth = SocketAddr::new(config.rpc_host.into(), config.rpc_port);
    let addr_no_auth = SocketAddr::new(config.rpc_host.into(), config.rpc_port_no_auth);
    let addr_ws = SocketAddr::new(config.rpc_host.into(), config.rpc_port_ws);
    let server = RpcServer::new(spine, config.sequencer_jwt(), rx_spawner, da_config);
    rt.spawn(server.run(addr_auth, addr_no_auth, addr_ws));
}

// TODO: jwt auth
// TODO: timing
#[derive(Debug, Clone)]
struct RpcServer {
    new_order_tx: Sender<Arc<Transaction>>,
    engine_timeout: Duration,
    engine_rpc_tx: Sender<EngineApi>,
    jwt: JwtSecret,
    telemetry_producer: Producer<TelemetryUpdate>,
    frag_receiver_spawner: tokio::sync::broadcast::Sender<SignedVersionedMessage>,
    da_config: OpDAConfig,
}

impl RpcServer {
    pub fn new<Db>(
        spine: &Spine<Db>,
        jwt: JwtSecret,
        frag_receiver_spawner: tokio::sync::broadcast::Sender<SignedVersionedMessage>,
        da_config: OpDAConfig,
    ) -> Self {
        Self {
            new_order_tx: spine.into(),
            engine_rpc_tx: spine.into(),
            engine_timeout: Duration::from_secs(1),
            jwt,
            telemetry_producer: telemetry_queue().into(),
            frag_receiver_spawner,
            da_config,
        }
    }

    #[tracing::instrument(skip_all, name = "rpc")]
    pub async fn run(self, addr_auth: SocketAddr, addr_no_auth: SocketAddr, addr_ws: SocketAddr) {
        info!(%addr_auth, "starting RPC server");
        let validator = JwtAuthValidator::new(self.jwt);
        let auth_layer = AuthLayer::new(validator);
        let service_builder = tower::ServiceBuilder::new().layer(auth_layer).timeout(std::time::Duration::from_secs(2));

        let server_auth = ServerBuilder::default()
            .set_config(
                ServerConfigBuilder::new().max_request_body_size(u32::MAX).max_response_body_size(u32::MAX).build(),
            )
            .set_http_middleware(service_builder)
            .build(addr_auth)
            .await
            .expect("failed to create eth RPC server");
        let mut module = MinimalEthApiServer::into_rpc(self.clone());
        module.merge(EngineApiServer::into_rpc(self.clone().clone())).expect("failed to merge modules");
        module.merge(ControlApiServer::into_rpc(self.clone())).expect("failed to merge modules");
        module.merge(OpMinerExtApiServer::into_rpc(self.clone())).expect("failed to merge modules");

        let server_handle_auth = server_auth.start(module);

        let service_builder = tower::ServiceBuilder::new().timeout(std::time::Duration::from_secs(2));

        let server_no_auth = ServerBuilder::default()
            .set_config(
                ServerConfigBuilder::new().max_request_body_size(u32::MAX).max_response_body_size(u32::MAX).build(),
            )
            .set_http_middleware(service_builder)
            .build(addr_no_auth)
            .await
            .expect("failed to create eth RPC server");
        let mut module = FabricGatewayApiServer::into_rpc(self.clone());
        module.merge(MinimalEthApiServer::into_rpc(self.clone())).expect("failed to merge modules");
        let server_handle_no_auth = server_no_auth.start(module);

        let state = StreamState { frags_tx: self.frag_receiver_spawner.clone() };
        let router = Router::new().route(STATE_STREAM_PATH, get(state_stream)).with_state(state);

        let listener = TcpListener::bind(addr_ws).await.expect("failed to bind to WS port");
        let ws_server = tokio::spawn(async move {
            match axum::serve(listener, router.into_make_service()).await {
                Ok(_) => info!("ws server exited"),
                Err(err) => error!(%err, "ws server exited"),
            }
        });

        //TODO: Handle other communcation from sequencer ?
        //      Idea: we have this part do rpc requests, using the rpc->sequencer channel,
        //      but we make it part of another sync actor that uses the connections and gathers
        //      state etc in a spinloop that the rpc runtime can use to serve requests with?
        server_handle_auth.stopped().await;
        server_handle_no_auth.stopped().await;
        let _ = ws_server.await;

        error!("server stopped");
    }
}

#[async_trait]
impl MinimalEthApiServer for RpcServer {
    #[tracing::instrument(skip_all, err, ret(level = Level::TRACE))]
    async fn send_raw_transaction(&self, bytes: Bytes) -> RpcResult<B256> {
        trace!(?bytes, "new request");

        let tx = Arc::new(Transaction::decode(bytes)?);
        TelemetryUpdate::send_ref(tx.uuid, tx.to_ingested_telemetry(), &self.telemetry_producer);

        let hash = tx.tx_hash();
        let _ = self.new_order_tx.send(tx.into());

        Ok(hash)
    }
}

#[async_trait]
impl ControlApiServer for RpcServer {
    async fn heartbeat(&self) -> RpcResult<()> {
        Ok(())
    }
}

#[async_trait]
impl OpMinerExtApiServer for RpcServer {
    async fn set_max_da_size(&self, max_tx_size: U64, max_block_size: U64) -> RpcResult<bool> {
        info!(target: "rpc", "Setting max DA size: tx={}, block={}", max_tx_size, max_block_size);
        let max_tx_size = max_tx_size.try_into().unwrap();
        let max_block_size = max_block_size.try_into().unwrap();
        self.da_config.set_max_da_size(max_tx_size, max_block_size);
        Ok(true)
    }
}
