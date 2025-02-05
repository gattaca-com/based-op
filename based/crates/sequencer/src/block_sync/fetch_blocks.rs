use std::time::Duration;

use alloy_consensus::Block;
use alloy_rpc_types::Block as RpcBlock;
use bop_common::{
    communication::{messages::BlockSyncMessage, Sender, SendersSpine, TrackedSenders},
    rpc::{RpcParam, RpcRequest, RpcResponse},
};
use bop_db::DatabaseRead;
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
pub async fn async_fetch_blocks_and_send_sequentially<Db: DatabaseRead>(
    mut curr_block: u64,
    end_block: u64,
    url: Url,
    block_sender: &SendersSpine<Db>,
    client: &Client,
) {
    tracing::info!("Fetching blocks from {}..={}", curr_block, end_block);

    let futures = (curr_block..=end_block).map(|i| fetch_block(i, client, url.clone()));

    let mut blocks: Vec<BlockWithSenders<OpBlock>> = join_all(futures).await;

    for block in blocks {
        block_sender.send_forever(block);
    }
    tracing::info!("Fetching and sending blocks done. Last fetched block: {}", curr_block - 1);
}

pub async fn fetch_block(block_number: u64, client: &Client, url: Url) -> BlockSyncMessage {
    let r = RpcRequest {
        jsonrpc: "2.0",
        method: "eth_getBlockByNumber",
        params: vec![RpcParam::String(format!("0x{block_number:x}")), RpcParam::Bool(true)],
        id: 1,
    };
    let req = client.post(url).json(&r);

    let mut backoff_ms = 10;
    let mut last_err = None;
    loop {
        let Ok(req) = req.try_clone().unwrap().send().await else {
            continue;
        };
        match req.json::<RpcResponse<RpcBlock<OpTxEnvelope>>>().await {
            Ok(block) => return convert_block(block.result),
            Err(err) => {
                tracing::warn!(
                    error=?err,
                    retry_after=?backoff_ms,
                    block=%block_number,
                    "RPC error while fetching block"
                );
                tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
                backoff_ms = std::cmp::min(backoff_ms * 2, 1000);
                last_err = Some(err);
            }
        }
    }
    unreachable!()
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
    use bop_common::communication::Spine;
    use bop_db::alloy_db::AlloyDB;
    use crossbeam_channel::bounded;

    use super::*;

    #[tokio::test]
    async fn test_single_block_fetch() {
        let client = Client::builder().timeout(Duration::from_secs(5)).build().expect("Failed to build HTTP client");

        let block = fetch_block(25738473, &client, TEST_BASE_RPC_URL.parse().unwrap()).await;

        assert_eq!(block.header.number, 25738473);
        assert_eq!(block.header.hash_slow(), b256!("ad9e6c25e60e711e5e99684892848adc06d44b1cc0e5056b06fcead6c7eb6186"));

        assert!(!block.body.transactions.is_empty());
        assert!(block.body.transactions.first().unwrap().is_deposit());
    }

    // #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    // async fn test_batch_fetch_ordering() {
    //     let start_block = 25738473;
    //     let end_block = 25738483;

    //     let spine: Spine<AlloyDB> = Spine::default();
    //     let connections = spine.to_connections("test");

    //     tokio::spawn({
    //         let client = reqwest::Client::new();
    //         let senders = connections.senders();
    //         async move {
    //             async_fetch_blocks_and_send_sequentially(
    //                 start_block,
    //                 end_block,
    //                 TEST_BASE_RPC_URL.parse().unwrap(),
    //                 senders,
    //                 &client,
    //             );
    //         }
    //     });

    //     let mut prev_block_num = start_block - 1;
    //     let mut blocks_received = 0;

    //     connections.receive(|block: BlockWithSenders<OpBlock>, _| {
    //         blocks_received += 1;

    //         assert!(block.header.number > prev_block_num, "Blocks must be in ascending order");
    //         prev_block_num = block.header.number;
    //     });

    //     assert_eq!(
    //         blocks_received,
    //         (end_block - start_block + 1) as usize,
    //         "Should receive exactly {} blocks",
    //         end_block - start_block + 1
    //     );
    //     assert_eq!(prev_block_num, end_block, "Last block should be end_block");
    // }
}
