use std::sync::Arc;

use bop_common::{
    actor::{Actor, ActorConfig},
    communication::{verify_or_remove_queue_files, Spine},
    config::GatewayArgs,
    db::DBFrag,
    time::Duration,
    utils::{init_tracing, wait_for_signal},
};
use bop_db::{init_database, DatabaseRead};
use bop_rpc::{start_rpc,  start_mock_engine_rpc};
use bop_sequencer::{block_sync::{block_fetcher::BlockFetcher, mock_fetcher::MockFetcher}, Sequencer, SequencerConfig};
use bop_simulator::Simulator;
use clap::Parser;
use tokio::runtime::Runtime;
use tracing::{error, info};

fn main() {
    if std::env::var_os("RUST_BACKTRACE").is_none() {
        std::env::set_var("RUST_BACKTRACE", "1");
    }

    let args = GatewayArgs::parse();
    verify_or_remove_queue_files();

    let _guards = init_tracing(None, 100, None);

    match run(args) {
        Ok(_) => {
            info!("gateway stopped");
        }

        Err(e) => {
            error!("{}", e);
            eprintln!("{}", e);
            std::process::exit(1);
        }
    }
}

fn run(args: GatewayArgs) -> eyre::Result<()> {
    info!("starting gateway");

    let spine = Spine::default();

    let db_bop = init_database(
        args.db_datadir.clone(),
        args.max_cached_accounts,
        args.max_cached_storages,
        args.chain_spec.clone(),
    )?;
    let db_frag: DBFrag<_> = db_bop.clone().into();
    let start_fetch = db_bop.head_block_number().expect("couldn't get head block number");
    let fetch_until =args.tmp_end_block;

    std::thread::scope(|s| {
        let rt: Arc<Runtime> = tokio::runtime::Builder::new_current_thread()
            .worker_threads(10)
            .enable_all()
            .build()
            .expect("failed to create runtime")
            .into();

        s.spawn({
            let db_frag = db_frag.clone();
            let rt = rt.clone();
            start_rpc(&args, &spine, db_frag, &rt);
            move || rt.block_on(wait_for_signal())
        });

        let sequencer = Sequencer::new(db_bop, db_frag.clone(), (&args).into());
        s.spawn(|| {
            sequencer.run(spine.to_connections("Sequencer"), ActorConfig::default().with_core(0));
        });

        s.spawn(|| {
            MockFetcher::new(args.rpc_fallback_url, start_fetch, fetch_until).run(
                spine.to_connections("BlockFetch"),
                ActorConfig::default().with_core(1).with_min_loop_duration(Duration::from_millis(10)),
            );
        });

        for core in 2..5 {
            let connections = spine.to_connections(format!("Simulator-{core}"));
            s.spawn({
                let db_frag = db_frag.clone();
                move || {
                    Simulator::create_and_run(connections, db_frag, ActorConfig::default().with_core(core));
                }
            });
        }
    });

    Ok(())
}
