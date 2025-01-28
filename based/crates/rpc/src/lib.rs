use bop_common::{communication::Sender, config::Config, db::DB, order::Order, rpc::EngineApiMessage, runtime::spawn};
use engine::EngineRpcServer;
use eth::EthRpcServer;

mod engine;
mod eth;

pub fn start_engine_rpc(config: &Config, engine_rpc_tx: Sender<EngineApiMessage>) {
    let server = EngineRpcServer::new(engine_rpc_tx, config.engine_api_timeout);
    spawn(server.run(config.engine_api_addr));
}

pub async fn start_eth_rpc(config: &Config, new_order_tx: Sender<Order>, db: DB) {
    let server = EthRpcServer::new(new_order_tx, db);
    spawn(server.run(config.eth_api_addr));
}
