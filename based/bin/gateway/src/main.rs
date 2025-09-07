use std::sync::Arc;

use bop_common::{
    actor::{Actor, ActorConfig},
    communication::Spine,
    config::GatewayArgs,
    shared::SharedState,
    signing::ECDSASigner,
    time::Duration,
    utils::{init_tracing, wait_for_signal},
};
use bop_db::{DatabaseRead, DatabaseWrite as _, init_database};
use bop_rpc::{gossiper::Gossiper, start_rpc};
use bop_sequencer::{
    Sequencer, SequencerConfig, Simulator,
    block_sync::{block_fetcher::BlockFetcher, mock_fetcher::MockFetcher, replay_fetcher::ReplayFetcher},
};
use clap::Parser;
use revm_primitives::B256;
use tokio::runtime::Runtime;
use tracing::{error, info, warn};

fn main() {
    if std::env::var_os("RUST_BACKTRACE").is_none() {
        unsafe { std::env::set_var("RUST_BACKTRACE", "1") };
    }

    let args = GatewayArgs::parse();
    let _guards = init_tracing((&args).into());
    bop_common::communication::verify_or_remove_queue_files();

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

    let db_bop =
        init_database(args.db_datadir.clone(), args.max_cached_accounts, args.max_cached_storages, args.chain.clone())?;

    let db_block = db_bop.head_block_number()?;
    let db_hash = db_bop.head_block_hash()?;

    info!(db_block, %db_hash, "starting gateway");

    let shared_state = SharedState::new(db_bop.clone().into());
    let head_block_number = db_bop.head_block_number()?;
    let start_fetch = if db_bop.head_block_hash()? == B256::ZERO {
        // genesis
        head_block_number
    } else {
        head_block_number + 1
    };
    let sequencer_config: SequencerConfig = (&args).into();
    let evm_config = sequencer_config.evm_config.clone();
    let (frag_broadcast_tx, _) = tokio::sync::broadcast::channel(10_000);

    if let Some(ref range) = args.replay_args.replay_blocks_range {
        warn!(?range, "Replay range found. Performing DB rollback if necessary");
        // Example: if start is 1000, then we should rollback to 998
        while db_bop.head_block_number()? > range.start().saturating_sub(2) {
            // FIXME: last write doesn't really work it seems, maybe because of the inner
            // self.reset_provider(); That is, reading with op-reth seems that only one block is
            // dropped.
            db_bop.roll_back_head()?;
        }
        info!(db_block = db_bop.head_block_number()?, db_hash = ?db_bop.head_block_hash()?, "New database head");
    }

    std::thread::scope(|s| {
        let rt: Arc<Runtime> = tokio::runtime::Builder::new_current_thread()
            .worker_threads(10)
            .enable_all()
            .build()
            .expect("failed to create runtime")
            .into();

        s.spawn({
            let rt = rt.clone();
            start_rpc(&args, &spine, &rt, frag_broadcast_tx.clone());
            move || rt.block_on(wait_for_signal())
        });

        s.spawn(|| {
            Sequencer::new(db_bop, shared_state.clone(), sequencer_config)
                .run(spine.to_connections("Sequencer"), ActorConfig::default());
        });

        // if args.replay_args.is_some() {
        // s.spawn(|| {
        //     let l2_el_verifier_url =
        //         args.replay_args.clone().expect("replay args").l2_el_verifier_url.expect("l2 el");
        //     let blocks_range =
        //         args.replay_args.expect("replay args").replay_blocks_range.expect("replay blocks range");
        //     ReplayFetcher::new(db_block, args.eth_client_url, l2_el_verifier_url, blocks_range).run(
        //         spine.to_connections("BlockFetch"),
        //         ActorConfig::default().with_min_loop_duration(Duration::from_millis(10)),
        //     );
        // });
        if let Some(mode) = args.mock {
            s.spawn(|| {
                MockFetcher::new(
                    args.eth_client_url,
                    start_fetch,
                    start_fetch + 100,
                    shared_state.as_ref().clone(),
                    mode,
                )
                .run(
                    spine.to_connections("BlockFetch"),
                    ActorConfig::default().with_min_loop_duration(Duration::from_millis(10)),
                );
            });
        } else {
            s.spawn(|| {
                BlockFetcher::new(args.eth_client_url, db_block).run(
                    spine.to_connections("BlockFetch"),
                    ActorConfig::default().with_min_loop_duration(Duration::from_millis(10)),
                );
            });
        }

        let root_peer_url = args.gossip_root_peer_url.clone();
        let gossip_signer_private_key = args.gossip_signer_private_key.map(|key| ECDSASigner::new(key).unwrap());
        s.spawn(|| {
            Gossiper::new(root_peer_url, gossip_signer_private_key, frag_broadcast_tx).run(
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
                    let simulator = Simulator::new(db_frag, evm_config, id, args.allow_reverts);
                    simulator.run(connections, ActorConfig::default());
                }
            });
        }
    });
    Ok(())
}
