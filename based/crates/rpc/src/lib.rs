use bop_common::{communication::Spine, config::Config};
use bop_db::DbStub;
use engine::EngineRpcServer;
use eth::EthRpcServer;
use tokio::runtime::Runtime;

mod engine;
mod eth;

pub fn start_engine_rpc(config: &Config, spine: &Spine, rt: &Runtime) {
    let server = EngineRpcServer::new(spine, config.engine_api_timeout);
    rt.spawn(server.run(config.engine_api_addr));
}

pub fn start_eth_rpc(config: &Config, spine: &Spine, db: DbStub, rt: &Runtime) {
    let server = EthRpcServer::new(spine, db);
    rt.spawn(server.run(config.eth_api_addr));
}
