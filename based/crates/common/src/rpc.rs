use alloy_primitives::B256;
use alloy_rpc_types::engine::{ExecutionPayloadV3, ForkchoiceState, ForkchoiceUpdated, PayloadId, PayloadStatus};
use jsonrpsee::types::{ErrorCode, ErrorObject as RpcErrorObject};
use op_alloy_rpc_types_engine::{OpExecutionPayloadEnvelopeV3, OpPayloadAttributes};
use tokio::sync::oneshot;

/// Supported Engine API RPC methods
pub enum EngineApiMessage {
    ForkChoiceUpdatedV3 {
        fork_choice_state:  ForkchoiceState,
        payload_attributes: Option<Box<OpPayloadAttributes>>,
        res_tx:             oneshot::Sender<ForkchoiceUpdated>,
    },
    NewPayloadV3 {
        payload:                  ExecutionPayloadV3,
        versioned_hashes:         Vec<B256>,
        parent_beacon_block_root: B256,
        res_tx:                   oneshot::Sender<PayloadStatus>,
    },
    GetPayloadV3 {
        payload_id: PayloadId,
        res:        oneshot::Sender<OpExecutionPayloadEnvelopeV3>,
    },
}

pub type RpcResult<T> = Result<T, RpcError>;

#[derive(Debug, thiserror::Error)]
pub enum RpcError {
    #[error("timeout")]
    Timeout(#[from] tokio::time::error::Elapsed),

    #[error("response channel closed {0}")]
    ChannelClosed(#[from] oneshot::error::RecvError),
}

impl From<RpcError> for RpcErrorObject<'static> {
    fn from(value: RpcError) -> Self {
        match value {
            RpcError::Timeout(_) | RpcError::ChannelClosed(_) => internal_error(),
        }
    }
}

fn internal_error() -> RpcErrorObject<'static> {
    RpcErrorObject::owned(ErrorCode::InternalError.code(), ErrorCode::InternalError.message(), None::<()>)
}
