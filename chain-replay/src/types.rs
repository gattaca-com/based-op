//! Types and type utils to convert between types.

use alloy::rpc::types::Block;

use op_alloy_consensus::OpBlock;
use op_alloy_rpc_types_engine::{OpExecutionPayload, OpExecutionPayloadEnvelope};

pub fn execution_payload_envelope_from_block(
    block: Block<op_alloy_rpc_types::Transaction>,
) -> OpExecutionPayloadEnvelope {
    let hash = block.hash();
    let parent_beacon_block_root = block.header.parent_beacon_block_root;

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

    let execution_payload = OpExecutionPayload::from_block_unchecked(hash, &op_block).0;

    assert_eq!(hash, execution_payload.block_hash());

    OpExecutionPayloadEnvelope { execution_payload, parent_beacon_block_root }
}
