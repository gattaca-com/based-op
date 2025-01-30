use std::sync::Arc;

use bop_common::{
    actor::Actor,
    communication::Spine,
    config::Config,
    time::Duration,
    utils::{init_tracing, wait_for_signal},
};
use bop_db::init_database;
use bop_rpc::{start_engine_rpc, start_eth_rpc};
use bop_sequencer::Sequencer;
use bop_simulator::Simulator;
use tokio::runtime::Runtime;

fn main() {
    let _guards = init_tracing(Some("gateway"), 100, None);

    let spine = Spine::default();
    let spine_c = spine.clone();

    let rpc_config = Config::default();

    let db = init_database("./").expect("can't run");
    let db_c = db.clone();

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
        let sim_0 = Simulator::new(0);
        sim_0.run(s, &spine, Some(Duration::from_micros(100)), Some(1));
        let sim_1 = Simulator::new(1);
        sim_1.run(s, &spine, Some(Duration::from_micros(100)), Some(2));
        let sim_2 = Simulator::new(2);
        // Ok to also run on 1 as it is sleeping for quite some time if there's no work to be done
        sim_2.run(s, &spine, Some(Duration::from_micros(100)), Some(1));

        let sequencer = Sequencer::new(db, rt_c);
        sequencer.run(s, &spine, None, Some(3));
    });
}
