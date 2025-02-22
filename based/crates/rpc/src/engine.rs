use alloy_primitives::B256;
use alloy_rpc_types::engine::{ExecutionPayloadV3, ForkchoiceState, ForkchoiceUpdated, PayloadId, PayloadStatus};
use bop_common::{
    api::EngineApiServer,
    communication::messages::{self, RpcError, RpcResult},
};
use jsonrpsee::core::async_trait;
use op_alloy_rpc_types_engine::{OpExecutionPayloadEnvelopeV3, OpPayloadAttributes};
use tokio::sync::oneshot;
use tracing::{trace, Level};

use crate::RpcServer;

impl RpcServer {
    fn send(&self, msg: messages::EngineApi) {
        let _ = self.engine_rpc_tx.send(msg.into());
    }
}

#[async_trait]
impl EngineApiServer for RpcServer {
    #[tracing::instrument(skip_all, ret(level = Level::TRACE))]
    async fn fork_choice_updated_v3(
        &self,
        fork_choice_state: ForkchoiceState,
        payload_attributes: Option<OpPayloadAttributes>,
    ) -> RpcResult<ForkchoiceUpdated> {
        trace!(?fork_choice_state, ?payload_attributes, "new request");

        self.send(messages::EngineApi::ForkChoiceUpdatedV3 {
            fork_choice_state,
            payload_attributes: payload_attributes.map(Box::new),
        });
        Err(RpcError::NoReturn)
    }

    #[tracing::instrument(skip_all,  ret(level = Level::TRACE))]
    async fn new_payload_v3(
        &self,
        payload: ExecutionPayloadV3,
        versioned_hashes: Vec<B256>,
        parent_beacon_block_root: B256,
    ) -> RpcResult<PayloadStatus> {
        trace!(?payload, ?versioned_hashes, %parent_beacon_block_root, "new request");

        self.send(messages::EngineApi::NewPayloadV3 { payload, versioned_hashes, parent_beacon_block_root });
        Err(RpcError::NoReturn)
    }

    #[tracing::instrument(skip_all, ret(level = Level::TRACE))]
    async fn get_payload_v3(&self, payload_id: PayloadId) -> RpcResult<OpExecutionPayloadEnvelopeV3> {
        trace!(%payload_id, "new request");

        let (tx, rx) = oneshot::channel();
        self.send(messages::EngineApi::GetPayloadV3 { payload_id, res: tx });

        // wait with timeout
        let res = tokio::time::timeout(self.engine_timeout.into(), rx).await??;

        Ok(res)
    }
}
