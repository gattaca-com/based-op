use alloy::{
    eips::Encodable2718,
    primitives::{Bytes, U256},
    rpc::types::{
        Block,
        engine::{ExecutionPayloadV1, ExecutionPayloadV2, ExecutionPayloadV3},
    },
};
use alloy_consensus::constants::EMPTY_WITHDRAWALS;

use op_alloy_rpc_types_engine::OpExecutionPayloadV4;

pub fn execution_payload_v4_from_block(
    block: Block<op_alloy_rpc_types::Transaction>,
) -> OpExecutionPayloadV4 {
    let transactions = block
        .transactions
        .as_transactions()
        .expect("full txs")
        .iter()
        .map(|tx| tx.inner.inner.encoded_2718())
        .map(Bytes::from)
        .collect();

    let v1 = ExecutionPayloadV1 {
        parent_hash: block.header.parent_hash,
        fee_recipient: block.header.beneficiary,
        state_root: block.header.state_root,
        receipts_root: block.header.receipts_root,
        logs_bloom: block.header.logs_bloom,
        prev_randao: block.header.mix_hash,
        block_number: block.header.number,
        gas_limit: block.header.gas_limit,
        gas_used: block.header.gas_used,
        timestamp: block.header.timestamp,
        extra_data: block.header.extra_data.clone(),
        base_fee_per_gas: U256::from(block.header.base_fee_per_gas.unwrap_or_default()),
        block_hash: block.header.hash,
        transactions,
    };
    let v2 = ExecutionPayloadV2 {
        payload_inner: v1,
        withdrawals: Default::default(),
    };
    let v3 = ExecutionPayloadV3 {
        payload_inner: v2,
        blob_gas_used: Default::default(),
        excess_blob_gas: Default::default(),
    };
    OpExecutionPayloadV4 {
        payload_inner: v3,
        withdrawals_root: block.header.withdrawals_root.unwrap_or(EMPTY_WITHDRAWALS),
    }
}
