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
use bop_rpc::start_rpc;
use bop_sequencer::{
    block_sync::{block_fetcher::BlockFetcher, mock_fetcher::MockFetcher},
    Sequencer, SequencerConfig, Simulator,
};
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
    let spine = Spine::default();

    let db = init_database(
        args.db_datadir.clone(),
        args.max_cached_accounts,
        args.max_cached_storages,
        args.chain_spec.clone(),
    )?;
    let db_head_block = db.head_block_number()?;
    let db_head_hash = db.head_block_hash()?;

    tracing::info!(db_head_block, %db_head_hash, "starting gateway");

    let db_frag: DBFrag<_> = db.clone().into();
    let sequencer_config: SequencerConfig = (&args).into();
    let evm_config = sequencer_config.evm_config.clone();

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

        s.spawn(|| {
            Sequencer::new(db, db_frag.clone(), sequencer_config)
                .run(spine.to_connections("Sequencer"), ActorConfig::default().with_core(0));
        });

        if args.test {
            s.spawn(|| {
                let start_fetch = db_head_block + 1;
                MockFetcher::new(args.rpc_fallback_url, start_fetch, start_fetch + 100).run(
                    spine.to_connections("BlockFetch"),
                    ActorConfig::default().with_core(1).with_min_loop_duration(Duration::from_millis(10)),
                );
            });
        } else {
            s.spawn(|| {
                BlockFetcher::new(args.rpc_fallback_url, db_head_block).run(
                    spine.to_connections("BlockFetch"),
                    ActorConfig::default().with_core(1).with_min_loop_duration(Duration::from_millis(10)),
                );
            });
        }

        for core in 2..5 {
            let connections = spine.to_connections(format!("sim-{core}"));
            s.spawn({
                let db_frag = db_frag.clone();
                let evm_config_c = evm_config.clone();
                move || {
                    let simulator = Simulator::new(db_frag, &evm_config_c, core);
                    simulator.run(connections, ActorConfig::default().with_core(core));
                }
            });
        }
    });

    Ok(())
}
