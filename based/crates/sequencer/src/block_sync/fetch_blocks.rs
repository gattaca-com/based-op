use std::time::Duration;

use alloy_rpc_types::Block;
use bop_common::{
    communication::Sender,
    rpc::{RpcParam, RpcRequest, RpcResponse},
};
use futures::future::join_all;
use reqwest::Client;
use tokio::{runtime::Runtime, task::JoinHandle};

/// Fetches a range of blocks sends them through the channel.
///
/// The fetching is done in batches, as soon as one batch is received fully, it is ordered sequentially by block number
/// and pushed in that order to the block sync.
///
/// curr_block/end_block are inclusive
pub fn fetch_blocks_and_send_sequentially(
    curr_block: u64,
    end_block: u64,
    url: String,
    block_sender: Sender<Result<Block, reqwest::Error>>,
    runtime: &Runtime,
) -> JoinHandle<()> {
    runtime.spawn(async move {
        async_fetch_blocks_and_send_sequentially(curr_block, end_block, url, block_sender).await;
    })
}

async fn async_fetch_blocks_and_send_sequentially(
    mut curr_block: u64,
    end_block: u64,
    url: String,
    block_sender: Sender<Result<Block, reqwest::Error>>,
) {
    const BATCH_SIZE: u64 = 20;

    tracing::info!("Fetching blocks from {}..={}", curr_block, end_block);
    let client = Client::builder().timeout(Duration::from_secs(5)).build().expect("Failed to build HTTP client");

    while curr_block <= end_block {
        let batch_end = (curr_block + BATCH_SIZE - 1).min(end_block);
        let futures = (curr_block..=batch_end).map(|i| fetch_block(i, &client, &url));

        let mut blocks: Vec<Result<Block, reqwest::Error>> = join_all(futures).await;

        // If any fail, send them first so block sync can handle errors.
        blocks.sort_unstable_by_key(|res| res.as_ref().map_or(0, |block| block.header.number));
        for block in blocks {
            let _ = block_sender.send(block.into());
        }

        curr_block = batch_end + 1;
    }

    tracing::info!("Fetching and sending blocks done. Last fetched block: {}", curr_block - 1);
}

async fn fetch_block(block_number: u64, client: &Client, url: &str) -> Result<Block, reqwest::Error> {
    const MAX_RETRIES: u32 = 10;

    let r = RpcRequest {
        jsonrpc: "2.0",
        method: "eth_getBlockByNumber",
        params: vec![RpcParam::String(format!("0x{block_number:x}")), RpcParam::Bool(true)],
        id: 1,
    };
    let req = client.post(url).json(&r);

    let mut backoff_ms = 10;
    let mut last_err = None;
    for retry in 0..MAX_RETRIES {
        match req.try_clone().unwrap().send().await?.json::<RpcResponse<Block>>().await {
            Ok(block) => return Ok(block.result),
            Err(err) => {
                tracing::warn!(
                    error=?err,
                    retry=retry,
                    retry_after=?backoff_ms,
                    "RPC error while fetching block"
                );
                tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
                backoff_ms = std::cmp::min(backoff_ms * 2, 1000);
                last_err = Some(err);
            }
        }
    }

    Err(last_err.unwrap())
}
