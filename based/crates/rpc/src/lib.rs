use std::{net::SocketAddr, sync::Arc};

use bop_common::{
    api::{EngineApiServer, EthApiServer, MinimalEthApiServer},
    communication::{messages::EngineApi, Sender, Spine},
    config::GatewayArgs,
    db::DatabaseRead,
    shared::SharedState,
    time::Duration,
    transaction::Transaction,
};
use jsonrpsee::{client_transport::ws::Url, http_client::HttpClient as RpcClient, server::ServerBuilder};
use tokio::runtime::Runtime;
use tracing::{error, info};

mod engine;
mod eth;
pub mod gossiper;

pub fn start_rpc<Db: DatabaseRead>(
    config: &GatewayArgs,
    spine: &Spine<Db>,
    shared_state: SharedState<Db>,
    rt: &Runtime,
) {
    let addr = SocketAddr::new(config.rpc_host.into(), config.rpc_port);
    let server = RpcServer::new(spine, shared_state, config.rpc_fallback_url.clone());
    rt.spawn(server.run(addr));
}

// TODO: jwt auth
// TODO: timing
#[derive(Debug, Clone)]
struct RpcServer<Db> {
    new_order_tx: Sender<Arc<Transaction>>,
    shared_state: SharedState<Db>,
    // TODO: this is a temporary fallback while we dont have a gossip to share state, in practice we should not serve
    // state directly from the gateway, and should only receive transactions
    fallback: RpcClient,
    engine_timeout: Duration,
    engine_rpc_tx: Sender<EngineApi>,
}

impl<Db: DatabaseRead> RpcServer<Db> {
    pub fn new(spine: &Spine<Db>, shared_state: SharedState<Db>, fallback_url: Url) -> Self {
        let fallback = RpcClient::builder().build(fallback_url).expect("failed building fallback rpc client");
        Self {
            new_order_tx: spine.into(),
            shared_state,
            fallback,
            engine_rpc_tx: spine.into(),
            engine_timeout: Duration::from_secs(1),
        }
    }

    #[tracing::instrument(skip_all, name = "rpc")]
    pub async fn run(self, addr: SocketAddr) {
        info!(%addr, "starting RPC server");

        let server = ServerBuilder::default().build(addr).await.expect("failed to create eth RPC server");
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
