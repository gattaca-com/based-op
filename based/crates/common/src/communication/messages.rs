use std::ops::{Deref, DerefMut};

use alloy_primitives::B256;
use alloy_rpc_types::engine::{ExecutionPayloadV3, ForkchoiceState, ForkchoiceUpdated, PayloadId, PayloadStatus};
use jsonrpsee::types::{ErrorCode, ErrorObject as RpcErrorObject};
use op_alloy_rpc_types_engine::{OpExecutionPayloadEnvelopeV3, OpPayloadAttributes};
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;

use crate::time::{Duration, IngestionTime, Instant, Nanos};

#[derive(Clone, Copy, Debug, PartialEq, PartialOrd, Serialize, Deserialize, Default)]
pub struct InternalMessage<T> {
    ingestion_t: IngestionTime,
    data: T,
}

impl<T> InternalMessage<T> {
    #[inline]
    pub fn new(ingestion_t: IngestionTime, data: T) -> Self {
        Self { ingestion_t, data }
    }

    #[inline]
    pub fn with_data<D>(&self, data: D) -> InternalMessage<D> {
        InternalMessage::new(self.ingestion_t, data)
    }

    #[inline]
    pub fn data(&self) -> &T {
        &self.data
    }

    #[inline]
    pub fn into_data(self) -> T {
        self.data
    }

    #[inline]
    pub fn map<R>(self, f: impl FnOnce(T) -> R) -> InternalMessage<R> {
        InternalMessage { ingestion_t: self.ingestion_t, data: f(self.data) }
    }

    #[inline]
    pub fn map_ref<R>(&self, f: impl FnOnce(&T) -> R) -> InternalMessage<R> {
        InternalMessage { ingestion_t: self.ingestion_t, data: f(&self.data) }
    }

    #[inline]
    pub fn unpack(self) -> (IngestionTime, T) {
        (self.ingestion_t, self.data)
    }

    /// This is only useful within the same socket as the original tsamp
    #[inline]
    pub fn elapsed(&self) -> Duration {
        self.ingestion_t.internal().elapsed()
    }

    /// These are real nanos since unix epoc
    #[inline]
    pub fn elapsed_nanos(&self) -> Nanos {
        self.ingestion_t.real().elapsed()
    }

    #[inline]
    pub fn ingestion_time(&self) -> IngestionTime {
        self.ingestion_t
    }
}

impl<T> From<InternalMessage<T>> for (IngestionTime, T) {
    #[inline]
    fn from(value: InternalMessage<T>) -> Self {
        value.unpack()
    }
}

impl<T> From<T> for InternalMessage<T> {
    #[inline]
    fn from(value: T) -> Self {
        Self::new(IngestionTime::now(), value)
    }
}

impl<T> Deref for InternalMessage<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.data
    }
}

impl<T> DerefMut for InternalMessage<T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.data
    }
}

impl<T> From<&InternalMessage<T>> for IngestionTime {
    #[inline]
    fn from(value: &InternalMessage<T>) -> Self {
        value.ingestion_t
    }
}

impl<T> AsRef<IngestionTime> for InternalMessage<T> {
    #[inline]
    fn as_ref(&self) -> &IngestionTime {
        &self.ingestion_t
    }
}

impl<T> From<&InternalMessage<T>> for Instant {
    #[inline]
    fn from(value: &InternalMessage<T>) -> Self {
        value.ingestion_t.into()
    }
}

impl<T> From<&InternalMessage<T>> for Nanos {
    #[inline]
    fn from(value: &InternalMessage<T>) -> Self {
        value.ingestion_t.into()
    }
}

impl<T> From<InternalMessage<T>> for Instant {
    #[inline]
    fn from(value: InternalMessage<T>) -> Self {
        value.ingestion_t.into()
    }
}

impl<T> From<InternalMessage<T>> for Nanos {
    #[inline]
    fn from(value: InternalMessage<T>) -> Self {
        value.ingestion_t.into()
    }
}

/// Supported Engine API RPC methods
#[derive(Debug)]
pub enum EngineApiMessage {
    ForkChoiceUpdatedV3 {
        fork_choice_state: ForkchoiceState,
        payload_attributes: Option<Box<OpPayloadAttributes>>,
        res_tx: oneshot::Sender<ForkchoiceUpdated>,
    },
    NewPayloadV3 {
        payload: ExecutionPayloadV3,
        versioned_hashes: Vec<B256>,
        parent_beacon_block_root: B256,
        res_tx: oneshot::Sender<PayloadStatus>,
    },
    GetPayloadV3 {
        payload_id: PayloadId,
        res: oneshot::Sender<OpExecutionPayloadEnvelopeV3>,
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SequencerToSimulator {
    Ping,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SimulatorToSequencer {
    Pong(usize),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SequencerToRpc {}
