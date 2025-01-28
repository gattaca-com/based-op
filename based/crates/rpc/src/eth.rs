use std::net::SocketAddr;

use bop_common::{
    communication::{messages::EthApi, Sender, Spine},
    time::Duration,
};
use tokio::time::sleep;

//TODO: @ltitanb
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct EthRpcServer {
    sender: Sender<EthApi>,
    timeout: Duration,
}
impl EthRpcServer {
    pub fn new(spine: &Spine, timeout: Duration) -> Self {
        Self { sender: spine.into(), timeout }
    }

    #[tracing::instrument(skip_all, name = "rpc:eth")]
    pub async fn run(self, addr: SocketAddr) {
        tracing::info!(%addr, "starting Eth RPC server");
        loop {
            tracing::info!("sending tx");
            if let Err(e) = self.sender.send(EthApi::random().into()) {
                tracing::error!("issue sending eth msg: {e}");
            };
            sleep(std::time::Duration::from_millis(50)).await
        }

        // let server = ServerBuilder::default().build(addr).await.expect("failed to create engine RPC server");
        // let execution_module = EngineApiServer::into_rpc(self);

        // let server_handle = server.start(execution_module);
        //TODO: Handle other communcation from sequencer ?
        //      Idea: we have this part do rpc requests, using the rpc->sequencer channel,
        //      but we make it part of another sync actor that uses the connections and gathers
        //      state etc in a spinloop that the rpc runtime can use to serve requests with?
        // server_handle.stopped().await;

        // tracing::error!("Eth RPC server stopped");
    }
}
