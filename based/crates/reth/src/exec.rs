use std::future::Future;

use alloy_primitives::{B256, BlockNumber};
use alloy_rpc_types::{Block, Log, TransactionReceipt};
use bop_common::p2p::{EnvV0, FragV0};

use crate::{error::ExecError, unsealed_block::UnsealedBlock};

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
    fn ensure_env(&mut self, env: &EnvV0) -> impl Future<Output = Result<(), ExecError>> + Send + '_;

    /// Execute all txs in `frag` on top of current overlay state.
    ///
    /// MUST be cumulative: txs execute after all previous frags's txs.
    fn execute_frag(
        &mut self,
        ub: &UnsealedBlock,
        frag: &FragV0,
    ) -> impl Future<Output = Result<ExecOutput, ExecError>> + Send + '_;

    fn seal(&mut self, ub: &UnsealedBlock) -> impl Future<Output = Result<(), ExecError>> + Send + '_;

    fn set_canonical(&mut self, b: &Block) -> impl Future<Output = Result<(), ExecError>> + Send + '_;

    fn get_block(&self, hash: B256, number: BlockNumber) -> impl Future<Output = Result<Block, ExecError>> + Send + '_;

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
    fn ensure_env(&mut self, _env: &EnvV0) -> impl Future<Output = Result<(), ExecError>> + Send + '_ {
        async move { Ok(()) }
    }

    fn execute_frag(
        &mut self,
        _ub: &UnsealedBlock,
        _frag: &FragV0,
    ) -> impl Future<Output = Result<ExecOutput, ExecError>> + Send + '_ {
        async move { Ok(ExecOutput { receipts: vec![], logs: vec![], gas_used_delta: 0 }) }
    }

    fn seal(&mut self, _ub: &UnsealedBlock) -> impl Future<Output = Result<(), ExecError>> + Send + '_ {
        async move { Ok(()) }
    }

    fn set_canonical(&mut self, _b: &Block) -> impl Future<Output = Result<(), ExecError>> + Send + '_ {
        async move { Ok(()) }
    }

    fn get_block(
        &self,
        _hash: B256,
        _number: BlockNumber,
    ) -> impl Future<Output = Result<Block, ExecError>> + Send + '_ {
        async move { Ok(Block::default()) }
    }

    fn reset(&mut self) {}
}
