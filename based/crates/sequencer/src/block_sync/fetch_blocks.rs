use std::time::Duration;

use alloy_consensus::Block;
use alloy_rpc_types::Block as RpcBlock;
use bop_common::{
    communication::{messages::BlockSyncMessage, Sender},
    rpc::{RpcParam, RpcRequest, RpcResponse},
};
use futures::future::join_all;
use op_alloy_consensus::OpTxEnvelope;
use reqwest::{Client, Url};
use reth_optimism_primitives::{OpBlock, OpTransactionSigned};
use reth_primitives::BlockWithSenders;
use reth_primitives_traits::SignedTransaction;
use tokio::{runtime::Runtime, task::JoinHandle};

#[allow(unused)]
pub(crate) const TEST_BASE_RPC_URL: &str = "https://base-rpc.publicnode.com";
#[allow(unused)]
pub(crate) const TEST_BASE_SEPOLIA_RPC_URL: &str = "https://base-sepolia-rpc.publicnode.com";

/// Fetches a range of blocks sends them through the channel.
///
/// The fetching is done in batches, as soon as one batch is received fully, it is ordered sequentially by block number
/// and pushed in that order to the block sync.
///
/// curr_block/end_block are inclusive
///
/// We first send all the previous blocks, then we send the last one if last_block is Some
pub async fn async_fetch_blocks_and_send_sequentially(
    mut curr_block: u64,
    end_block: u64,
    url: Url,
    block_sender: Sender<BlockSyncMessage>,
    last_block: Option<BlockSyncMessage>,
) {
    const BATCH_SIZE: u64 = 20;

    tracing::info!("Fetching blocks from {}..={}", curr_block, end_block);
    let client = Client::builder().timeout(Duration::from_secs(5)).build().expect("Failed to build HTTP client");

    while curr_block <= end_block {
        let batch_end = (curr_block + BATCH_SIZE - 1).min(end_block);
        let futures = (curr_block..=batch_end).map(|i| fetch_block(i, &client, url.clone()));

        let mut blocks: Vec<Result<BlockWithSenders<OpBlock>, reqwest::Error>> = join_all(futures).await;

        // If any fail, send them first so block sync can handle errors.
        blocks.sort_unstable_by_key(|res| res.as_ref().map_or(0, |block| block.header.number));
        for block in blocks {
            let mut block = block.map_err(|e| e.into()).into();
            while let Err(b) = block_sender.send(block) {
                block = b.into_inner();
            }
        }

        curr_block = batch_end + 1;
    }

    tracing::info!("Fetching and sending blocks done. Last fetched block: {}", curr_block - 1);
    if let Some(cur_block) = last_block {
        tracing::info!("Sending current block");
        block_sender.send(cur_block.into()).expect("couldn't send block sync");
    }
}

pub async fn fetch_block(
    block_number: u64,
    client: &Client,
    url: Url,
) -> Result<BlockWithSenders<OpBlock>, reqwest::Error> {
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
        match req.try_clone().unwrap().send().await?.json::<RpcResponse<RpcBlock<OpTxEnvelope>>>().await {
            Ok(block) => return Ok(convert_block(block.result)),
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

/// Converts an RPC block with OpTxEnvelope transactions to a consensus block with OpTransactionSigned
pub fn convert_block(block: RpcBlock<OpTxEnvelope>) -> BlockWithSenders<OpBlock> {
    // First convert the block to consensus format
    let consensus_block = block.into_consensus();

    // Now convert the transactions
    let mut recovery_buf = Vec::with_capacity(200);
    let (converted_txs, senders): (Vec<_>, Vec<_>) = consensus_block
        .body
        .transactions
        .into_iter()
        .map(|tx| {
            let signed_tx = OpTransactionSigned::from_envelope(tx);
            recovery_buf.clear(); // Reuse buffer for next transaction
            let sender = signed_tx
                .recover_signer_unchecked_with_buf(&mut recovery_buf)
                .expect("transaction signature must be valid");
            (signed_tx, sender)
        })
        .unzip();

    let block = Block {
        header: consensus_block.header,
        body: alloy_consensus::BlockBody {
            transactions: converted_txs,
            ommers: consensus_block.body.ommers,
            withdrawals: consensus_block.body.withdrawals,
        },
    };

    BlockWithSenders::new_unchecked(block, senders)
}

#[cfg(test)]
mod tests {
    use alloy_primitives::b256;
    use crossbeam_channel::bounded;

    use super::*;

    #[tokio::test]
    async fn test_single_block_fetch() {
        let client = Client::builder().timeout(Duration::from_secs(5)).build().expect("Failed to build HTTP client");

        let block = fetch_block(25738473, &client, TEST_BASE_RPC_URL.parse().unwrap()).await.unwrap();

        assert_eq!(block.header.number, 25738473);
        assert_eq!(block.header.hash_slow(), b256!("ad9e6c25e60e711e5e99684892848adc06d44b1cc0e5056b06fcead6c7eb6186"));

        assert!(!block.body.transactions.is_empty());
        assert!(block.body.transactions.first().unwrap().is_deposit());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_batch_fetch_ordering() {
        let (sender, receiver) = bounded(100);

        let start_block = 25738473;
        let end_block = 25738483;

        tokio::spawn(async_fetch_blocks_and_send_sequentially(
            start_block,
            end_block,
            TEST_BASE_RPC_URL.parse().unwrap(),
            sender,
            None,
        ));

        let mut prev_block_num = start_block - 1;
        let mut blocks_received = 0;

        while let Ok(block_result) = receiver.recv() {
            let block = block_result.data().as_ref().unwrap();
            blocks_received += 1;

            assert!(block.header.number > prev_block_num, "Blocks must be in ascending order");
            prev_block_num = block.header.number;

            if blocks_received == (end_block - start_block + 1) as usize {
                break;
            }
        }

        assert_eq!(
            blocks_received,
            (end_block - start_block + 1) as usize,
            "Should receive exactly {} blocks",
            end_block - start_block + 1
        );
        assert_eq!(prev_block_num, end_block, "Last block should be end_block");
    }
}
