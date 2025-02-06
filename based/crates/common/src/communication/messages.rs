use std::{
    ops::{Deref, DerefMut},
    sync::Arc,
};

use alloy_consensus::Header;
use alloy_eips::eip2718::{Decodable2718, Encodable2718};
use alloy_primitives::B256;
use alloy_rpc_types::engine::{
    ExecutionPayloadV1, ExecutionPayloadV2, ExecutionPayloadV3, ForkchoiceState, ForkchoiceUpdated, PayloadAttributes,
    PayloadError, PayloadId, PayloadStatus,
};
use jsonrpsee::types::{ErrorCode, ErrorObject as RpcErrorObject};
use op_alloy_rpc_types_engine::{OpExecutionPayloadEnvelopeV3, OpPayloadAttributes};
use reth_evm::{execute::BlockExecutionError, NextBlockEnvAttributes};
use reth_optimism_primitives::{OpBlock, OpTransactionSigned};
use reth_primitives::BlockWithSenders;
use revm::{CacheState, DatabaseRef};
use revm_primitives::{Address, EVMError, EvmState, U256};
use serde::{Deserialize, Serialize};
use strum_macros::AsRefStr;
use thiserror::Error;
use tokio::sync::oneshot::{self, Receiver};

use crate::{
    db::{DBFrag, DBSorting, DatabaseRead},
    time::{Duration, IngestionTime, Instant, Nanos},
    transaction::{SimulatedTx, Transaction},
};

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
#[derive(Debug, AsRefStr)]
pub enum EngineApi {
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
impl EngineApi {
    pub fn messages_from_block(
        block: &BlockSyncMessage,
        txs_in_attributes: bool,
        no_tx_pool: Option<bool>,
    ) -> (Receiver<PayloadStatus>, EngineApi, Receiver<ForkchoiceUpdated>, EngineApi, EngineApi) {
        let (new_payload_tx, new_payload_rx) = oneshot::channel();
        let block_hash = block.hash_slow();
        let transactions = block
            .body
            .transactions
            .iter()
            .map(|t| {
                let mut buf = vec![];
                t.encode_2718(&mut buf);
                buf.into()
            })
            .collect::<Vec<_>>();
        let payload_attributes = PayloadAttributes {
            timestamp: block.timestamp,
            prev_randao: block.mix_hash,
            suggested_fee_recipient: block.beneficiary,
            withdrawals: Default::default(),
            parent_beacon_block_root: block.parent_beacon_block_root,
        };
        let op_payload_attributes = Some(Box::new(OpPayloadAttributes {
            payload_attributes,
            transactions: txs_in_attributes.then(|| transactions.clone()),
            no_tx_pool,
            gas_limit: Some(block.gas_limit),
            eip_1559_params: None,
        }));
        let v1 = ExecutionPayloadV1 {
            parent_hash: block.parent_hash,
            fee_recipient: block.beneficiary,
            state_root: block.state_root,
            receipts_root: block.receipts_root,
            logs_bloom: block.logs_bloom,
            prev_randao: block.mix_hash,
            block_number: block.number,
            gas_limit: block.gas_limit,
            gas_used: block.gas_used,
            timestamp: block.timestamp,
            extra_data: block.extra_data.clone(),
            base_fee_per_gas: U256::from(block.base_fee_per_gas.unwrap_or_default()),
            block_hash,
            transactions,
        };
        let v2 = ExecutionPayloadV2 { payload_inner: v1, withdrawals: Default::default() };
        let v3 = ExecutionPayloadV3 {
            payload_inner: v2,
            blob_gas_used: Default::default(),
            excess_blob_gas: Default::default(),
        };
        let new_payload = EngineApi::NewPayloadV3 {
            payload: v3,
            versioned_hashes: Default::default(), // not used
            parent_beacon_block_root: block.parent_beacon_block_root.unwrap(),
            res_tx: new_payload_tx,
        };
        let (fcu_tx, _fcu_rx) = oneshot::channel();

        let fcu_1 = EngineApi::ForkChoiceUpdatedV3 {
            fork_choice_state: ForkchoiceState {
                head_block_hash: block_hash,
                safe_block_hash: Default::default(),
                finalized_block_hash: Default::default(),
            },
            payload_attributes: None,
            res_tx: fcu_tx,
        };
        let (fcu_tx, fcu_rx) = oneshot::channel();
        let fcu = EngineApi::ForkChoiceUpdatedV3 {
            fork_choice_state: ForkchoiceState {
                head_block_hash: block_hash,
                safe_block_hash: Default::default(),
                finalized_block_hash: Default::default(),
            },
            payload_attributes: op_payload_attributes,
            res_tx: fcu_tx,
        };
        (new_payload_rx, new_payload, fcu_rx, fcu_1, fcu)
    }
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

    #[error("db error: {0}")]
    Db(#[from] crate::db::Error),
}

impl From<RpcError> for RpcErrorObject<'static> {
    fn from(value: RpcError) -> Self {
        match value {
            RpcError::Internal |
            RpcError::Timeout(_) |
            RpcError::ChannelClosed(_) |
            RpcError::Jsonrpsee(_) |
            RpcError::TokioJoin(_) |
            RpcError::Db(_) => internal_error(),
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

#[derive(Clone, Debug, AsRefStr)]
#[repr(u8)]
pub enum SequencerToSimulator<Db> {
    /// Simulate Tx on top of a partially built frag
    SimulateTx(Arc<Transaction>, Arc<DBSorting<Db>>),
    /// Simulate Tx Top of frag
    //TODO: Db could be set on frag commit once we broadcast msgs to sims
    SimulateTxTof(Arc<Transaction>, DBFrag<Db>),
}

#[derive(Debug)]
pub struct SimulatorToSequencer {
    /// Sender address and nonce
    pub sender_info: (Address, u64),
    pub state_id: u64,
    pub msg: SimulatorToSequencerMsg,
}

impl SimulatorToSequencer {
    pub fn new(sender_info: (Address, u64), state_id: u64, msg: SimulatorToSequencerMsg) -> Self {
        Self { sender_info, state_id, msg }
    }

    pub fn sender(&self) -> &Address {
        &self.sender_info.0
    }

    pub fn nonce(&self) -> u64 {
        self.sender_info.1
    }
}

pub type SimulationResult<T> = Result<T, SimulationError>;

#[derive(Clone, Debug)]
pub struct TopOfBlockResult {
    pub cache_state: CacheState,
    pub forced_inclusion_txs: Vec<SimulatedTx>,
}
impl TopOfBlockResult {
    pub fn gas_used(&self) -> u64 {
        self.forced_inclusion_txs.iter().map(|t| t.gas_used()).sum()
    }
    pub fn payment(&self) -> U256 {
        self.forced_inclusion_txs.iter().map(|t| t.payment).sum()
    }
}

#[derive(Debug, AsRefStr)]
#[repr(u8)]
pub enum SimulatorToSequencerMsg {
    /// Simulation on top of any state.
    Tx(SimulationResult<SimulatedTx>),
    /// Simulation on top of a fragment. Used by the transaction pool.
    TxPoolTopOfFrag(SimulationResult<SimulatedTx>),
    /// Top of block sims are done, we can start sequencing
    TopOfBlock(TopOfBlockResult),
}

#[derive(Clone, Debug, Error, AsRefStr)]
#[repr(u8)]
pub enum SimulationError {
    #[error("Evm error: {0}")]
    EvmError(String),
    #[error("Order pays nothing")]
    ZeroPayment,
}

#[derive(Clone, Copy, Debug, PartialEq, AsRefStr)]
pub enum SequencerToExternal {}

#[derive(Debug, thiserror::Error)]
pub enum BlockSyncError {
    #[error("Block fetch failed: {0}")]
    Fetch(#[from] reqwest::Error),
    #[error("Block execution failed: {0}")]
    Execution(#[from] BlockExecutionError),
    #[error("DB error: {0}")]
    Database(#[from] crate::db::Error),
    #[error("Payload error: {0}")]
    Payload(#[from] PayloadError),
    #[error("Failed to recover transaction signer")]
    SignerRecovery,
}

pub type BlockSyncMessage = BlockWithSenders<OpBlock>;

#[derive(Clone, Debug, AsRefStr)]
pub enum BlockFetch {
    FromTo(u64, u64),
}

/// Represents the parameters required to configure the next block.
#[derive(Clone, Debug)]
pub struct EvmBlockParams<Db: 'static> {
    pub parent_header: Header,
    pub attributes: NextBlockAttributes,
    /// New frag db
    pub db: DBFrag<Db>,
}

#[derive(Clone)]
pub struct NextBlockAttributes {
    pub env_attributes: NextBlockEnvAttributes,
    /// Txs to add top of block.
    pub forced_inclusion_txs: Vec<Arc<Transaction>>,
    /// Parent block beacon root.
    pub parent_beacon_block_root: Option<B256>,
}

impl std::fmt::Debug for NextBlockAttributes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NextBlockAttributes")
            .field("env_attributes", &self.env_attributes)
            .field("forced_inclusion_txs", &self.forced_inclusion_txs.len())
            .field("parent_beacon_block_root", &self.parent_beacon_block_root)
            .finish()
    }
}
