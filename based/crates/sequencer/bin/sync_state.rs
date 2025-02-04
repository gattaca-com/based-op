use std::path::PathBuf;

use bop_common::{runtime::RuntimeOrHandle, time::Duration, utils::initialize_test_tracing};
use bop_db::{init_database, BopDB, BopDbRead};
use bop_sequencer::block_sync::{
    fetch_blocks::{async_fetch_blocks_and_send_sequentially, fetch_block},
    BlockSync,
};
use clap::Parser;
use reqwest::{Client, Url};
use reth_optimism_chainspec::BASE_SEPOLIA;
use tracing::level_filters::LevelFilter;

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

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() -> eyre::Result<()> {
    initialize_test_tracing(LevelFilter::INFO);

    let args = Args::parse();
    let rpc_url = Url::parse(&args.rpc_url)?;
    let rpc_url_clone = rpc_url.clone();

    let db: bop_db::DB = init_database(&args.db_path, 1000, 1000)?;
    let db_head = db.readonly().unwrap().block_number()?;
    let head_state_root = db.state_root()?;

    tracing::info!("Starting sync. From block: {db_head} to block: {}. State root: {head_state_root:?}", args.end_block);

    let chain_spec = BASE_SEPOLIA.clone();
    let handle: RuntimeOrHandle = tokio::runtime::Handle::current().into();
    let mut block_sync = BlockSync::new(chain_spec, handle, rpc_url.clone());

    // Start block fetching task.
    let (sender, block_receiver) = crossbeam_channel::unbounded();
    tokio::spawn(async move {
        async_fetch_blocks_and_send_sequentially(db_head + 1, args.end_block, rpc_url, sender, None).await;
    });

    // Fetch prev state root for checks
    let client = Client::builder().timeout(Duration::from_secs(5).into()).build().expect("Failed to build HTTP client");
    let mut rpc_head_block_state_root = fetch_block(db_head, &client, rpc_url_clone).await.unwrap().header.state_root;

    while let Ok(block) = block_receiver.recv() {
        let block = block.into_data().expect("issue receiving block");

        // Check head state root matches block state root.
        let head_state_root = db.state_root()?;
        if head_state_root != rpc_head_block_state_root {
            panic!("Head state root does not match block state root. Head: {head_state_root:?}, Block: {rpc_head_block_state_root:?}");
        }
        tracing::info!(
            "Head state root matches block state root. Head: {head_state_root:?}, Block: {rpc_head_block_state_root:?}"
        );
        rpc_head_block_state_root = block.header.state_root;

        block_sync.apply_and_commit_block(&block, &db, true).unwrap();
    }

    Ok(())
}
