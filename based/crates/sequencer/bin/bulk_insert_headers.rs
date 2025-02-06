use std::path::PathBuf;

use alloy_provider::ProviderBuilder;
use bop_db::init_database;
use bop_sequencer::block_sync::fetch_blocks::fetch_block;
use clap::Parser;
use reqwest::Url;
use reth_db::{
    tables,
    transaction::{DbTx, DbTxMut},
};
use reth_optimism_chainspec::BASE_SEPOLIA;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Path to the database directory
    #[arg(short, long)]
    db_path: PathBuf,

    /// Start block number
    #[arg(short, long)]
    start_block: u64,

    /// End block number
    #[arg(short, long)]
    end_block: u64,

    /// RPC URL
    #[arg(short, long)]
    rpc_url: Url,
}

// start = 21127040
// end = 21127055

#[tokio::main]
async fn main() -> eyre::Result<()> {
    let args = Args::parse();

    // Initialize DB
    let db = init_database(&args.db_path, 0, 0, BASE_SEPOLIA.clone())?;

    // Initialize HTTP client with reasonable timeouts
    let provider = ProviderBuilder::new().network().on_http(args.rpc_url);

    // Fetch and wait for all blocks
    let mut batch_futures = Vec::with_capacity((args.end_block - args.start_block + 1) as usize);
    for block_num in args.start_block..=args.end_block {
        batch_futures.push(fetch_block(block_num, &provider));
    }
    let mut blocks = futures::future::join_all(batch_futures).await.into_iter().collect::<Vec<_>>();

    // Bulk insert the batch
    if blocks.is_empty() {
        return Err(eyre::eyre!("No blocks to insert"));
    }

    let tx = db.factory.provider_rw()?;
    for block in blocks.drain(..) {
        let hash = block.header.hash_slow();
        println!("Inserting block. Number: {}, Hash: {}", block.header.number, hash);
        tx.tx_ref()
            .put::<tables::CanonicalHeaders>(block.header.number, hash)
            .map_err(|e| eyre::eyre!("Failed to insert header: {e}"))?;

        // Now get the inserted header to check it's correct
        let inserted_header = tx
            .tx_ref()
            .get::<tables::CanonicalHeaders>(block.header.number)
            .map_err(|e| eyre::eyre!("Failed to get inserted header: {e}"))?;
        if inserted_header != Some(hash) {
            return Err(eyre::eyre!("Inserted header is incorrect. Got: {:?}, Expected: {:?}", inserted_header, hash));
        } else {
            println!("Inserted header is correct. Got: {:?}, Expected: {:?}", inserted_header, hash);
        }
    }

    tx.commit().map_err(|e| eyre::eyre!("Failed to commit transaction: {e}"))?;

    println!("Completed block insertion from {} to {}", args.start_block, args.end_block);
    Ok(())
}
