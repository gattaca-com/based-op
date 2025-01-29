use std::net::SocketAddr;

use alloy_primitives::B256;
use alloy_rpc_types::engine::{ExecutionPayloadV3, ForkchoiceState, ForkchoiceUpdated, PayloadId, PayloadStatus};
use bop_common::{
    api::EngineApiServer,
    communication::{
        messages::{self, EngineApi, RpcResult},
        Sender, Spine,
    },
    time::Duration,
};
use jsonrpsee::{core::async_trait, server::ServerBuilder};
use op_alloy_rpc_types_engine::{OpExecutionPayloadEnvelopeV3, OpPayloadAttributes};
use tokio::sync::oneshot;
use tracing::{error, info, trace, Level};

// TODO: jwt auth
// TODO: timing
pub struct EngineRpcServer {
    timeout: Duration,
    engine_rpc_tx: Sender<EngineApi>,
}

impl EngineRpcServer {
    pub fn new(spine: &Spine, timeout: Duration) -> Self {
        Self { engine_rpc_tx: spine.into(), timeout }
    }

    #[tracing::instrument(skip_all, name = "rpc_engine")]
    pub async fn run(self, addr: SocketAddr) {
        info!(%addr, "starting RPC server");

        let server = ServerBuilder::default().build(addr).await.expect("failed to create engine RPC server");
        let execution_module = EngineApiServer::into_rpc(self);

        let server_handle = server.start(execution_module);
        //TODO: Handle other communcation from sequencer ?
        //      Idea: we have this part do rpc requests, using the rpc->sequencer channel,
        //      but we make it part of another sync actor that uses the connections and gathers
        //      state etc in a spinloop that the rpc runtime can use to serve requests with?
        server_handle.stopped().await;

        error!("server stopped");
    }

    fn send(&self, msg: messages::EngineApi) {
        let _ = self.engine_rpc_tx.send(msg.into());
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
        self.send(messages::EngineApi::ForkChoiceUpdatedV3 {
            fork_choice_state,
            payload_attributes: payload_attributes.map(Box::new),
            res_tx: tx,
        });

        // wait with timeout
        let res = tokio::time::timeout(self.timeout.into(), rx).await??;

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
        self.send(messages::EngineApi::NewPayloadV3 {
            payload,
            versioned_hashes,
            parent_beacon_block_root,
            res_tx: tx,
        });

        // wait with timeout
        let res = tokio::time::timeout(self.timeout.into(), rx).await??;

        Ok(res)
    }

    #[tracing::instrument(skip_all, err, ret(level = Level::TRACE))]
    async fn get_payload_v3(&self, payload_id: PayloadId) -> RpcResult<OpExecutionPayloadEnvelopeV3> {
        trace!(%payload_id, "new request");

        let (tx, rx) = oneshot::channel();
        self.send(messages::EngineApi::GetPayloadV3 { payload_id, res: tx });

        // wait with timeout
        let res = tokio::time::timeout(self.timeout.into(), rx).await??;

        Ok(res)
    }
}
