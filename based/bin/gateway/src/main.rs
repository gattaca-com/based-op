use std::sync::Arc;

use bop_common::{
    actor::{Actor, ActorConfig},
    communication::Spine,
    config::GatewayArgs,
    db::DBFrag,
    utils::{init_tracing, wait_for_signal},
};
use bop_db::init_database;
use bop_rpc::{start_mock_engine_rpc, start_rpc};
use bop_sequencer::{Sequencer, SequencerConfig};
use bop_simulator::Simulator;
use clap::Parser;
use tokio::runtime::Runtime;

fn main() {
    if std::env::var_os("RUST_BACKTRACE").is_none() {
        std::env::set_var("RUST_BACKTRACE", "1");
    }
    let args = GatewayArgs::parse();
    let _guards = init_tracing(None, 100, None);

    let spine = Spine::default();

    let db_bop = init_database(
        args.db_datadir.clone(),
        args.max_cached_accounts,
        args.max_cached_storages,
        args.chain_spec.clone(),
    )
    .expect("can't run");
    let db_frag: DBFrag<_> = db_bop.clone().into();

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
            let sequencer = Sequencer::new(db_bop, db_frag.clone(), rt, SequencerConfig::default_base_sepolia());
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

        start_mock_engine_rpc(&spine, args.tmp_end_block);
    });
}
