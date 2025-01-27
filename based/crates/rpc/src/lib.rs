use bop_common::{config::Config, rpc::EngineApiMessage, runtime::spawn};
use crossbeam_channel::Sender;
use engine::EngineRpcServer;

mod engine;

pub async fn start_engine_rpc(config: &Config, engine_rpc_tx: Sender<EngineApiMessage>) {
    let server = EngineRpcServer::new(engine_rpc_tx, config.engine_api_timeout);

    spawn(server.run(config.engine_api_addr));
}

pub async fn start_eth_rpc() {}
