use std::time::Duration;

use alloy_consensus::Block;
use alloy_provider::Provider;
use alloy_rpc_types::Block as RpcBlock;
use bop_common::communication::{messages::BlockSyncMessage, SendersSpine, TrackedSenders};
use bop_db::DatabaseRead;
use futures::future::join_all;
use reth_optimism_primitives::{OpBlock, OpTransactionSigned};
use reth_primitives::BlockWithSenders;
use reth_primitives_traits::SignedTransaction;
use tracing::{info, warn};

use super::AlloyProvider;

/// Fetches a range of blocks sends them through the channel.
///
/// The fetching is done in batches, as soon as one batch is received fully, it is ordered sequentially by block number
/// and pushed in that order to the block sync.
///
/// curr_block/end_block are inclusive
///
/// We first send all the previous blocks, then we send the last one if last_block is Some
pub async fn async_fetch_blocks_and_send_sequentially<Db: DatabaseRead>(
    curr_block: u64,
    end_block: u64,
    block_sender: &SendersSpine<Db>,
    provider: &AlloyProvider,
) {
    let futures = (curr_block..=end_block).map(|i| fetch_block(i, provider));

    let blocks: Vec<BlockWithSenders<OpBlock>> = join_all(futures).await;

    for block in blocks {
        block_sender.send_forever(block);
    }

    info!(start = curr_block, last = end_block, "fetched blocks");
}

pub async fn fetch_block(block_number: u64, client: &AlloyProvider) -> BlockSyncMessage {
    const BACKOFF_MAX: Duration = Duration::from_secs(1);
    const BACKOFF_STEP: Duration = Duration::from_millis(10);

    let mut backoff = BACKOFF_STEP;

    loop {
        match client.get_block_by_number(block_number.into(), true.into()).await {
            Ok(Some(block)) => return convert_block(block),

            Ok(None) => {
                warn!(?backoff, block_number, "block not found");
            }

            Err(err) => {
                warn!(?err, ?backoff, block_number, "failed fetching");
            }
        }

        tokio::time::sleep(backoff).await;
        backoff = std::cmp::min(backoff * 2, BACKOFF_MAX);
    }
}

/// Converts an RPC block with OpTxEnvelope transactions to a consensus block with OpTransactionSigned
pub fn convert_block(block: RpcBlock<op_alloy_rpc_types::Transaction>) -> BlockWithSenders<OpBlock> {
    // First convert the block to consensus format
    let consensus_block = block.into_consensus();

    // Now convert the transactions
    let mut recovery_buf = Vec::with_capacity(200);
    let (converted_txs, senders): (Vec<_>, Vec<_>) = consensus_block
        .body
        .transactions
        .into_iter()
        .map(|tx| {
            let signed_tx = OpTransactionSigned::from_envelope(tx.inner.inner);
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
pub const TEST_BASE_RPC_URL: &str = "https://base-rpc.publicnode.com";

#[cfg(test)]
pub const TEST_BASE_SEPOLIA_RPC_URL: &str = "https://base-sepolia-rpc.publicnode.com";

#[cfg(test)]
mod tests {
    use alloy_primitives::b256;
    use alloy_provider::ProviderBuilder;
    use bop_common::communication::Spine;
    use bop_db::AlloyDB;

    use super::*;

    #[ignore = "Requires RPC call"]
    #[tokio::test]
    async fn test_single_block_fetch() {
        let provider = ProviderBuilder::new().network().on_http(TEST_BASE_RPC_URL.parse().unwrap());
        let block = fetch_block(25738473, &provider).await;

        assert_eq!(block.header.number, 25738473);
        assert_eq!(block.header.hash_slow(), b256!("ad9e6c25e60e711e5e99684892848adc06d44b1cc0e5056b06fcead6c7eb6186"));

        assert!(!block.body.transactions.is_empty());
        assert!(block.body.transactions.first().unwrap().is_deposit());
    }

    #[ignore = "Requires RPC call"]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_batch_fetch_ordering() {
        let start_block = 25738473;
        let end_block = 25738483;

        let spine: Spine<AlloyDB> = Spine::default();
        let mut connections = spine.to_connections("test");
        let senders_clone = connections.senders().clone();

        let provider = ProviderBuilder::new().network().on_http(TEST_BASE_RPC_URL.parse().unwrap());
        let handle = tokio::spawn(async move {
            async_fetch_blocks_and_send_sequentially(start_block, end_block, &senders_clone, &provider).await;
        });

        let mut prev_block_num = start_block - 1;
        let mut blocks_received = 0;

        loop {
            connections.receive(|block: BlockWithSenders<OpBlock>, _| {
                blocks_received += 1;

                assert!(block.header.number > prev_block_num, "Blocks must be in ascending order");
                prev_block_num = block.header.number;
            });

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

        handle.abort();
    }
}
