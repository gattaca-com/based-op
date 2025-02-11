use std::sync::Arc;

use bop_common::{
    actor::{Actor, ActorConfig},
    communication::{verify_or_remove_queue_files, Spine},
    config::GatewayArgs,
    shared::SharedState,
    time::Duration,
    utils::{init_tracing, wait_for_signal},
};
use bop_db::{init_database, DatabaseRead};
use bop_rpc::{gossiper::Gossiper, start_rpc};
use bop_sequencer::{
    block_sync::{
        block_fetcher::BlockFetcher,
        mock_fetcher::{MockFetcher, Mode},
    },
    Sequencer, SequencerConfig, Simulator,
};
use clap::Parser;
use revm_primitives::B256;
use tokio::runtime::Runtime;
use tracing::{error, info};

fn main() {
    if std::env::var_os("RUST_BACKTRACE").is_none() {
        std::env::set_var("RUST_BACKTRACE", "1");
    }

    let args = GatewayArgs::parse();
    let _guards = init_tracing((&args).into());
    verify_or_remove_queue_files();

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

    let db_bop = init_database(
        args.db_datadir.clone(),
        args.max_cached_accounts,
        args.max_cached_storages,
        args.chain_spec.clone(),
    )?;

    let db_block = db_bop.head_block_number()?;
    let db_hash = db_bop.head_block_hash()?;

    info!(db_block, %db_hash, "starting gateway");

    let shared_state = SharedState::new(db_bop.clone().into());
    let head_block_number = db_bop.head_block_number().expect("couldn't get head block number");
    let start_fetch = if db_bop.head_block_hash().expect("couldn't get head block hash") == B256::ZERO {
        // genesis
        head_block_number
    } else {
        head_block_number + 1
    };
    let sequencer_config: SequencerConfig = (&args).into();
    let evm_config = sequencer_config.evm_config.clone();

    std::thread::scope(|s| {
        let rt: Arc<Runtime> = tokio::runtime::Builder::new_current_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("failed to create runtime")
            .into();

        s.spawn({
            let rt = rt.clone();
            start_rpc(&args, &spine, shared_state.clone(), &rt);
            move || rt.block_on(wait_for_signal())
        });

        let state_clone = shared_state.clone();
        s.spawn(|| {
            Sequencer::new(db_bop, state_clone, sequencer_config)
                .run(spine.to_connections("Sequencer"), ActorConfig::default());
        });

        let fragdb_clone = shared_state.as_ref().clone();
        if args.test {
            s.spawn(|| {
                MockFetcher::new(args.rpc_fallback_url, start_fetch, start_fetch + 100, fragdb_clone, Mode::Spammer)
                    .run(
                        spine.to_connections("BlockFetch"),
                        ActorConfig::default().with_min_loop_duration(Duration::from_millis(10)),
                    );
            });
        } else {
            s.spawn(|| {
                BlockFetcher::new(args.rpc_fallback_url, db_block).run(
                    spine.to_connections("BlockFetch"),
                    ActorConfig::default().with_min_loop_duration(Duration::from_millis(10)),
                );
            });
        }
        let root_peer_url = args.gossip_root_peer_url.clone();
        s.spawn(|| {
            Gossiper::new(root_peer_url).run(
                spine.to_connections("Gossiper"),
                ActorConfig::default().with_min_loop_duration(Duration::from_millis(10)),
            );
        });

        for id in 0..args.sim_threads {
            s.spawn({
                let evm_config = evm_config.clone();
                let connections = spine.to_connections(format!("Simulator-{id}"));
                let db_frag = (&shared_state).into();
                move || {
                    let simulator = Simulator::new(db_frag, &evm_config, id);
                    simulator.run(connections, ActorConfig::default());
                }
            });
        }
    });

    Ok(())
}
