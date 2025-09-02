//! Types and type utils to convert between types.

use alloy::rpc::types::Block;

use op_alloy_consensus::OpBlock;
use op_alloy_rpc_types_engine::OpExecutionPayload;

pub fn execution_payload_from_block(
    block: Block<op_alloy_rpc_types::Transaction>,
) -> OpExecutionPayload {
    let hash = block.hash();
    let txs = block
        .transactions
        .into_transactions_vec()
        .into_iter()
        .map(|t| t.inner.inner.into_inner())
        .collect::<Vec<_>>();

    let op_block: OpBlock = {
        alloy_consensus::BlockBody {
            transactions: txs,
            ommers: vec![],
            withdrawals: block.withdrawals,
        }
        .into_block(block.header.into_consensus())
    };

    OpExecutionPayload::from_block_unchecked(hash, &op_block).0
}
