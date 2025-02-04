use bop_common::{communication::Spine, config::Config, db::DBFrag};
use bop_db::BopDbRead;
use bop_rpc::eth::EthRpcServer;
use tokio::runtime::Runtime;


pub fn start_eth_rpc<Db: BopDbRead>(config: &Config, spine: &Spine<Db>, db: DBFrag<Db>, rt: &Runtime) {
    let server = EthRpcServer::new(spine, db);
    rt.spawn(server.run(config.eth_api_addr));
}
