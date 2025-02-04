use bop_common::{
    actor::{Actor, ActorConfig},
    communication::Spine,
    config::Config,
    db::{BopDbRead, DBFrag},
};
use engine::EngineRpcServer;
use engine_mock::MockEngineRpcServer;
use eth::EthRpcServer;
use tokio::runtime::Runtime;

mod engine;
mod engine_mock;
mod eth;

pub fn start_engine_rpc<Db: BopDbRead>(config: &Config, spine: &Spine<Db>, rt: &Runtime) {
    let server = EngineRpcServer::new(spine, config.engine_api_timeout);
    rt.spawn(server.run(config.engine_api_addr));
}

pub fn start_eth_rpc<Db: BopDbRead>(config: &Config, spine: &Spine<Db>, db: DBFrag<Db>, rt: &Runtime) {
    let server = EthRpcServer::new(spine, db, config.eth_fallback_url.clone());
    rt.spawn(server.run(config.eth_api_addr));
}

pub fn start_mock_engine_rpc<Db: BopDbRead>(spine: &Spine<Db>, last_block_number: u64) {
    let server = MockEngineRpcServer::new(last_block_number);
    server.run(spine.to_connections("MockEngineRpc"), ActorConfig::default());
}
