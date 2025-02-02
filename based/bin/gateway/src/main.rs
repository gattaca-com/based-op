use std::{net::Ipv4Addr, sync::Arc};

use bop_common::{
    actor::{Actor, ActorConfig},
    communication::Spine,
    config::Config,
    db::{init_database, BopDB, DBFrag},
    utils::{init_tracing, wait_for_signal},
};
use bop_rpc::{start_engine_rpc, start_eth_rpc};
use bop_sequencer::{Sequencer, SequencerConfig};
use bop_simulator::Simulator;
use tokio::runtime::Runtime;

fn main() {
    let _guards = init_tracing(Some("gateway"), 100, None);

    let spine = Spine::default();
    let spine_c = spine.clone();

    let rpc_config = get_config();

    // TODO values from config
    let max_cached_accounts = 10_000;
    let max_cached_storages = 100_000;

    let db_bop = init_database("./", max_cached_accounts, max_cached_storages).expect("can't run");
    let db_read: DBFrag<_> = db_bop.readonly().expect("Failed to create read-only DB").into();
    let db_c = db_read.clone();

    std::thread::scope(|s| {
        let rt: Arc<Runtime> = tokio::runtime::Builder::new_current_thread()
            .worker_threads(10)
            .enable_all()
            .build()
            .expect("failed to create runtime")
            .into();
        let rt_c = rt.clone();

        s.spawn(move || {
            start_engine_rpc(&rpc_config, &spine_c, &rt);
            start_eth_rpc(&rpc_config, &spine_c, db_c, &rt);

            rt.block_on(wait_for_signal())
        });
        let db_s = db_read.clone();
        s.spawn(|| {
            let sequencer = Sequencer::new(db_bop, db_s, rt_c, SequencerConfig::default());
            sequencer.run(spine.to_connections("Sequencer"), ActorConfig::default().with_core(0));
        });
        for (i, core) in (1..4).enumerate() {
            let db_sim = db_read.clone();
            let connections = spine.to_connections(format!("Simulator-{core}"));
            s.spawn(move || {
                Simulator::create_and_run(connections, db_sim, ActorConfig::default());
            });
        }
    });
}

// TODO: load from cli
fn get_config() -> Config {
    use std::net::SocketAddr;

    use bop_common::time::Duration;

    Config {
        engine_api_addr: SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), 8001),
        eth_api_addr: SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), 8002),
        engine_api_timeout: Duration::from_secs(1),
        eth_fallback_url: "http://todo.xyz".parse().unwrap(),
    }
}
