use std::{net::Ipv4Addr, path::PathBuf, sync::Arc};

use bop_common::{
    actor::{Actor, ActorConfig},
    communication::Spine,
    config::Config,
    db::{BopDB, DBFrag},
    utils::{init_tracing, wait_for_signal},
};
use bop_db::init_database;
use bop_rpc::{start_engine_rpc, start_eth_rpc, start_mock_engine_rpc};
use bop_sequencer::{Sequencer, SequencerConfig};
use bop_simulator::Simulator;
use clap::Parser;
use reth_optimism_chainspec::OpChainSpecBuilder;
use tokio::runtime::Runtime;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Path to the database directory
    #[arg(short, long)]
    db_path: PathBuf,

    /// Last block number to sync to, inclusive.
    #[arg(short, long)]
    end_block: u64,

    /// RPC URL, used to fetch blocks.
    #[arg(short, long)]
    rpc_url: String,
}

fn main() {
    let _guards = init_tracing(Some("gateway"), 100, None);

    let spine = Spine::default();
    let spine_c = spine.clone();

    let args = Args::parse();
    let mut config = SequencerConfig::with_chain_spec(OpChainSpecBuilder::base_sepolia().build());
    config.rpc_url = reqwest::Url::parse(&args.rpc_url).unwrap();

    let rpc_config = get_config();

    // TODO values from config
    let max_cached_accounts = 10_000;
    let max_cached_storages = 100_000;

    let db_bop = init_database(&args.db_path, max_cached_accounts, max_cached_storages).expect("can't run");
    let db_frag: DBFrag<_> = db_bop.readonly().expect("Failed to create read-only DB").into();

    std::thread::scope(|s| {
        let rt: Arc<Runtime> = tokio::runtime::Builder::new_current_thread()
            .worker_threads(10)
            .enable_all()
            .build()
            .expect("failed to create runtime")
            .into();
        let rt_c = rt.clone();

        s.spawn({
            let db_frag = db_frag.clone();

            move || {
                start_engine_rpc(&rpc_config, &spine_c, &rt);
                start_eth_rpc(&rpc_config, &spine_c, db_frag, &rt);

                rt.block_on(wait_for_signal())
            }
        });

        s.spawn(|| {
            let sequencer = Sequencer::new(db_bop, db_frag.clone(), rt_c, config);
            sequencer.run(spine.to_connections("Sequencer"), ActorConfig::default().with_core(0));
        });

        for core in 1..4 {
            let connections = spine.to_connections(format!("Simulator-{core}"));
            s.spawn({
                let db_frag = db_frag.clone();

                move || {
                    Simulator::create_and_run(connections, db_frag, ActorConfig::default());
                }
            });
        }

        start_mock_engine_rpc(&spine, args.end_block);
    });
}

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
