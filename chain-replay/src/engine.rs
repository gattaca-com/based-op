//! An engine client.

use alloy::{
    primitives::B256,
    rpc::types::engine::{
        ExecutionPayloadInputV2, ForkchoiceState, ForkchoiceUpdated, PayloadStatus,
    },
    transports::TransportResult,
};
use kona_engine::EngineClient;

use op_alloy_provider::ext::engine::OpEngineApi;
use op_alloy_rpc_types_engine::{OpExecutionPayload, OpPayloadAttributes};

pub trait EngineExt {
    async fn new_payload(
        &self,
        payload: OpExecutionPayload,
        parent_beacon_block_root: Option<B256>,
    ) -> TransportResult<PayloadStatus>;
    async fn fork_choice_update(
        &self,
        fork_choice_state: ForkchoiceState,
        payload_attributes: Option<OpPayloadAttributes>,
        is_v3: bool,
    ) -> TransportResult<ForkchoiceUpdated>;
}

impl EngineExt for EngineClient {
    async fn new_payload(
        &self,
        payload: OpExecutionPayload,
        parent_beacon_block_root: Option<B256>,
    ) -> TransportResult<PayloadStatus> {
        let call = match payload {
            OpExecutionPayload::V1(v1) => {
                let input = ExecutionPayloadInputV2 { execution_payload: v1, withdrawals: None };
                // It is also intended to use v2 but with nil withdrawals, as new_payload_v1 isn't
                // exposed.
                self.new_payload_v2(input)
            }
            OpExecutionPayload::V2(v2) => self.new_payload_v2(v2.into_payload_input_v2(true)),
            OpExecutionPayload::V3(v3) => {
                self.new_payload_v3(v3, parent_beacon_block_root.expect("parent_beacon_block_root"))
            }
            OpExecutionPayload::V4(v4) => {
                self.new_payload_v4(v4, parent_beacon_block_root.expect("parent_beacon_block_root"))
            }
        };

        call.await
    }

    async fn fork_choice_update(
        &self,
        fork_choice_state: ForkchoiceState,
        payload_attributes: Option<OpPayloadAttributes>,
        is_v3: bool,
    ) -> TransportResult<ForkchoiceUpdated> {
        let call = if is_v3 {
            self.fork_choice_updated_v3(fork_choice_state, payload_attributes)
        } else {
            self.fork_choice_updated_v2(fork_choice_state, payload_attributes)
        };

        call.await
    }
}
