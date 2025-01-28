use std::{net::SocketAddr, time::Duration};

use alloy_primitives::B256;
use alloy_rpc_types::engine::{ExecutionPayloadV3, ForkchoiceState, ForkchoiceUpdated, PayloadId, PayloadStatus};
use bop_common::{
    api::EngineApiServer,
    communication::messages::{EngineApiMessage, RpcResult},
};
use crossbeam_channel::Sender;
use jsonrpsee::{core::async_trait, server::ServerBuilder};
use op_alloy_rpc_types_engine::{OpExecutionPayloadEnvelopeV3, OpPayloadAttributes};
use tokio::sync::oneshot;
use tracing::{error, info, trace, Level};

// TODO: jwt auth
// TODO: timing
pub struct EngineRpcServer {
    engine_rpc_tx: Sender<EngineApiMessage>,
    timeout: Duration,
}

impl EngineRpcServer {
    pub fn new(engine_rpc_tx: Sender<EngineApiMessage>, timeout: Duration) -> Self {
        Self { engine_rpc_tx, timeout }
    }

    #[tracing::instrument(skip_all, name = "rpc:engine")]
    pub async fn run(self, addr: SocketAddr) {
        info!(%addr, "starting RPC server");

        let server = ServerBuilder::default().build(addr).await.expect("failed to create engine RPC server");
        let execution_module = EngineApiServer::into_rpc(self);

        let server_handle = server.start(execution_module);
        server_handle.stopped().await;

        error!("server stopped");
    }
}

#[async_trait]
impl EngineApiServer for EngineRpcServer {
    #[tracing::instrument(skip_all, err, ret(level = Level::TRACE))]
    async fn fork_choice_updated_v3(
        &self,
        fork_choice_state: ForkchoiceState,
        payload_attributes: Option<OpPayloadAttributes>,
    ) -> RpcResult<ForkchoiceUpdated> {
        trace!(?fork_choice_state, ?payload_attributes, "new request");

        let (tx, rx) = oneshot::channel();
        let _ = self.engine_rpc_tx.send(EngineApiMessage::ForkChoiceUpdatedV3 {
            fork_choice_state,
            payload_attributes: payload_attributes.map(Box::new),
            res_tx: tx,
        });

        // wait with timeout
        let res = tokio::time::timeout(self.timeout, rx).await??;

        Ok(res)
    }

    #[tracing::instrument(skip_all, err, ret(level = Level::TRACE))]
    async fn new_payload_v3(
        &self,
        payload: ExecutionPayloadV3,
        versioned_hashes: Vec<B256>,
        parent_beacon_block_root: B256,
    ) -> RpcResult<PayloadStatus> {
        trace!(?payload, ?versioned_hashes, %parent_beacon_block_root, "new request");

        let (tx, rx) = oneshot::channel();
        let _ = self.engine_rpc_tx.send(EngineApiMessage::NewPayloadV3 {
            payload,
            versioned_hashes,
            parent_beacon_block_root,
            res_tx: tx,
        });

        // wait with timeout
        let res = tokio::time::timeout(self.timeout, rx).await??;

        Ok(res)
    }

    #[tracing::instrument(skip_all, err, ret(level = Level::TRACE))]
    async fn get_payload_v3(&self, payload_id: PayloadId) -> RpcResult<OpExecutionPayloadEnvelopeV3> {
        trace!(%payload_id, "new request");

        let (tx, rx) = oneshot::channel();
        let _ = self.engine_rpc_tx.send(EngineApiMessage::GetPayloadV3 { payload_id, res: tx });

        // wait with timeout
        let res = tokio::time::timeout(self.timeout, rx).await??;

        Ok(res)
    }
}
