//! Types and type utils to convert between types.

use alloy::{
    primitives::FixedBytes,
    rpc::types::{Block, engine::PayloadAttributes},
};

use op_alloy_consensus::OpBlock;
use op_alloy_rpc_types_engine::{
    OpExecutionPayload, OpExecutionPayloadEnvelope, OpPayloadAttributes,
};

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

pub fn op_attributes_from_block(
    block: &Block<op_alloy_rpc_types::Transaction>,
) -> OpPayloadAttributes {
    let payload_attributes = PayloadAttributes {
        timestamp: block.header.timestamp,
        prev_randao: block.header.mix_hash,
        suggested_fee_recipient: block.header.beneficiary,
        withdrawals: Default::default(),
        parent_beacon_block_root: block.header.parent_beacon_block_root,
    };
    OpPayloadAttributes {
        payload_attributes,
        transactions: None,
        no_tx_pool: Some(true),
        gas_limit: Some(block.header.gas_limit),
        eip_1559_params: Some(FixedBytes::from_slice(&block.header.extra_data[1..9])),
    }
}
