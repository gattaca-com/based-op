use alloy_rpc_types::{Log, TransactionReceipt};
use bop_common::p2p::{EnvV0, FragV0};
use thiserror::Error;

use crate::unsealed_block::UnsealedBlock;

#[derive(Debug, Error)]
pub enum ExecError {
    #[error("executor not initialized")]
    NotInitialized,

    #[error("execution failed: {0}")]
    Failed(String),

    #[error("seal failed: {0}")]
    SealFailed(String),
}

#[derive(Debug, Clone)]
pub struct ExecOutput {
    pub receipts: Vec<TransactionReceipt>,
    pub logs: Vec<Log>,
    pub gas_used_delta: u64,
}

/// This trait is the ONLY place that needs to know about Reth internals.
/// Everything else is just state-machine + bookkeeping.
pub trait UnsealedExecutor: Send {
    /// Ensure the executor context is ready for this env (initialize overlay state, block env, etc.)
    fn ensure_env(&mut self, env: &EnvV0) -> Result<(), ExecError>;

    /// Execute all txs in `frag` on top of current overlay state.
    ///
    /// MUST be cumulative: txs execute after all previous frags' txs.
    async fn execute_frag(&mut self, ub: &UnsealedBlock, frag: &FragV0) -> Result<ExecOutput, ExecError>;

    /// Finalize (post-exec changes, compute roots if needed, etc.)
    async fn seal(&mut self, ub: &UnsealedBlock) -> Result<(), ExecError>;

    /// Reset overlay state completely.
    fn reset(&mut self);
}

/// Apply the executor output to the UnsealedBlock (common logic).
pub fn apply_exec_output(ub: &mut UnsealedBlock, out: ExecOutput) {
    ub.receipts.extend(out.receipts);
    ub.logs.extend(out.logs);
    ub.cumulative_gas_used = ub.cumulative_gas_used.saturating_add(out.gas_used_delta);
}

/// A very small “dummy” executor so you can compile & test state machine early.
/// Replace with Reth executor.
pub struct NoopExecutor;

impl UnsealedExecutor for NoopExecutor {
    fn ensure_env(&mut self, _env: &EnvV0) -> Result<(), ExecError> {
        Ok(())
    }

    async fn execute_frag(&mut self, _ub: &UnsealedBlock, _frag: &FragV0) -> Result<ExecOutput, ExecError> {
        Ok(ExecOutput { receipts: vec![], logs: vec![], gas_used_delta: 0 })
    }

    async fn seal(&mut self, _ub: &UnsealedBlock) -> Result<(), ExecError> {
        Ok(())
    }

    fn reset(&mut self) {}
}
