
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
    #[error("internal error")]
    Internal,

    #[error("timeout")]
    Timeout(#[from] tokio::time::error::Elapsed),

    #[error("response channel closed {0}")]
    ChannelClosed(#[from] oneshot::error::RecvError),

    #[error("invalid transaction bytes")]
    InvalidTransaction(#[from] alloy_rlp::Error),

    #[error("jsonrpsee error {0}")]
    Jsonrpsee(#[from] jsonrpsee::core::ClientError),

    #[error("join error: {0}")]
    TokioJoin(#[from] tokio::task::JoinError),
}

impl From<RpcError> for RpcErrorObject<'static> {
    fn from(value: RpcError) -> Self {
        match value {
            RpcError::Internal |
            RpcError::Timeout(_) |
            RpcError::ChannelClosed(_) |
            RpcError::Jsonrpsee(_) |
            RpcError::TokioJoin(_) => internal_error(),
            RpcError::InvalidTransaction(error) => RpcErrorObject::owned(
                ErrorCode::InvalidParams.code(),
                ErrorCode::InvalidParams.message(),
                Some(error.to_string()),
            ),
        }
    }
}

fn internal_error() -> RpcErrorObject<'static> {
    RpcErrorObject::owned(ErrorCode::InternalError.code(), ErrorCode::InternalError.message(), None::<()>)
}
