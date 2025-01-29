use bop_common::{communication::Spine, config::Config};
use bop_db::DbStub;
use engine::EngineRpcServer;
use eth::EthRpcServer;

mod engine;
mod eth;

pub fn start_engine_rpc(config: &Config, spine: &Spine) {
    let server = EngineRpcServer::new(spine, config.engine_api_timeout);
    tokio::spawn(server.run(config.engine_api_addr));
}

pub fn start_eth_rpc(config: &Config, spine: &Spine, db: DbStub) {
    let server = EthRpcServer::new(spine, db);
    tokio::spawn(server.run(config.eth_api_addr));
}
