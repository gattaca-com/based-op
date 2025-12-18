use alloy_consensus::crypto::RecoveryError;
use alloy_eips::eip2718::Eip2718Error;
use alloy_primitives::B256;
use op_alloy_consensus::EIP1559ParamError;
use reth_optimism_evm::OpBlockExecutionError;
use reth_optimism_rpc::OpEthApiError;
use reth_storage_errors::ProviderError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DriverError {
    #[error("failed to send command to driver task (task not running)")]
    DriverGone,

    #[error("failed to receive response from driver task (response dropped)")]
    ResponseDropped,

    #[error("driver not initialized, call env_v0 first")]
    NotInitialized,

    #[error(
        "cannot open a new unsealed block while there's one already in progress (current={current}, incoming={incoming})"
    )]
    UnsealedBlockInProgress { current: u64, incoming: u64 },

    #[error("seal mismatch: {what}")]
    SealMismatch { what: String },

    #[error(transparent)]
    Exec(#[from] ExecError),

    #[error(transparent)]
    Other(#[from] anyhow::Error),

    #[error(transparent)]
    UnsealedBlock(#[from] UnsealedBlockError),

    #[error(transparent)]
    ValidateSeal(#[from] ValidateSealError),
}

#[derive(Debug, Error)]
pub enum ValidateSealError {
    #[error("block hash mismatch, expected {expected:?}, got {got:?}")]
    BlockHashMismatch { expected: B256, got: B256 },

    #[error("parent hash mismatch, expected {expected:?}, got {got:?}")]
    ParentHashMismatch { expected: B256, got: B256 },

    #[error("state root mismatch, expected {expected:?}, got {got:?}")]
    StateRootMismatch { expected: B256, got: B256 },

    #[error("transactions root mismatch, expected {expected:?}, got {got:?}")]
    TransactionsRootMismatch { expected: B256, got: B256 },

    #[error("receipts root mismatch, expected {expected:?}, got {got:?}")]
    ReceiptsRootMismatch { expected: B256, got: B256 },

    #[error("gas used mismatch, expected {expected}, got {got}")]
    GasUsedMismatch { expected: u64, got: u64 },

    #[error("gas limit mismatch, expected {expected}, got {got}")]
    GasLimitMismatch { expected: u64, got: u64 },

    #[error("total frags mismatch, expected {expected}, got {got}")]
    TotalFragsMismatch { expected: u64, got: u64 },
}

#[derive(Debug, Error)]
pub enum UnsealedBlockError {
    #[error("failed to decode EIP-2718 tx at index {index}")]
    TxDecode {
        index: usize,
        #[source]
        source: Eip2718Error,
    },

    #[error("stale frag (older block): frag.block={frag_block} < env.number={env_number}")]
    StaleFrag { frag_block: u64, env_number: u64 },

    #[error("frag is not applicable to current unsealed env: frag.block={frag_block} env.number={env_number}")]
    WrongBlock { frag_block: u64, env_number: u64 },

    #[error("frag sequencing violation: expected next seq, got {got}, last={last:?}")]
    SeqMismatch { got: u64, last: Option<u64> },

    #[error("received frag after last frag already accepted")]
    AlreadyEnded,
}

#[derive(Debug, Error)]
pub enum ExecError {
    #[error("executor not initialized")]
    NotInitialized,

    #[error("cannot open a new unsealed block while there's one already in progress")]
    Inprogress,

    #[error("execution failed: {0}")]
    Failed(String),

    #[error("seal failed: {0}")]
    SealFailed(String),

    #[error(transparent)]
    StorageProvider(#[from] ProviderError),

    #[error(transparent)]
    OpBlockExecution(#[from] OpBlockExecutionError),

    #[error(transparent)]
    Recovery(#[from] RecoveryError),

    #[error(transparent)]
    Eip1559Param(#[from] EIP1559ParamError),

    #[error(transparent)]
    OpEthApi(#[from] OpEthApiError),
}
