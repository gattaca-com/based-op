use std::{net::SocketAddr, sync::Arc};

use alloy_provider::Provider;
use bop_common::{
    actor::Actor,
    communication::Spine,
    config::Config,
    time::Duration,
    utils::{init_tracing, wait_for_signal},
};
use bop_db::{BopDB, init_database};
use bop_rpc::{start_engine_rpc, start_eth_rpc};
use bop_sequencer::Sequencer;
use bop_simulator::Simulator;
use tokio::runtime::Runtime;

fn main() {
    let _guards = init_tracing(Some("gateway"), 100, None);

    let spine = Spine::default();
    let spine_c = spine.clone();

    let rpc_config = Config::default();

    // TODO values from config
    let max_cached_accounts = 10_000;
    let max_cached_storages = 100_000;

    let bop_db = init_database("./", max_cached_accounts, max_cached_storages).expect("can't run");
    let db = bop_db.readonly().expect("Failed to create read-only DB");
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

            rt.spawn(spam_rpc_txs(rpc_config.eth_api_addr));

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

async fn spam_rpc_txs(addr: SocketAddr) {
    use std::time::Duration;

    use alloy_eips::eip2718::Encodable2718;
    use alloy_provider::ProviderBuilder;
    use bop_common::transaction::Transaction;

    let url = format!("http://{}", addr).parse().unwrap();

    let provider = ProviderBuilder::new().on_http(url);

    loop {
        let tx = Transaction::random().tx.encoded_2718();
        let pending = provider.send_raw_transaction(&tx).await.expect("failed to send tx");
        let hash = pending.tx_hash();
        tracing::debug!(%hash, "sent tx");

        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
