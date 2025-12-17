use std::future::Future;

use alloy_primitives::{BlockNumber, B256};
use alloy_rpc_types::{Block, Log, TransactionReceipt};
use bop_common::p2p::{EnvV0, FragV0};

use crate::error::ExecError;
use crate::unsealed_block::UnsealedBlock;

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
    /// MUST be cumulative: txs execute after all previous frags's txs.
    fn execute_frag<'a>(
        &'a mut self,
        ub: &'a UnsealedBlock,
        frag: &'a FragV0,
    ) -> impl Future<Output = Result<ExecOutput, ExecError>> + Send + 'a;

    fn set_canonical<'a>(
        &'a mut self,
        b: &'a Block,
    ) -> impl Future<Output = Result<(), ExecError>> + Send + 'a;

    fn seal<'a>(
        &'a mut self,
        ub: &'a UnsealedBlock,
    ) -> impl Future<Output = Result<(), ExecError>> + Send + 'a;

    fn get_block<'a>(
        &'a self,
        hash: B256,
        number: BlockNumber,
    ) -> impl Future<Output = Result<Block, ExecError>> + Send + 'a;

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

    fn execute_frag<'a>(
        &'a mut self,
        _ub: &'a UnsealedBlock,
        _frag: &'a FragV0,
    ) -> impl Future<Output = Result<ExecOutput, ExecError>> + Send + 'a {
        async move { Ok(ExecOutput { receipts: vec![], logs: vec![], gas_used_delta: 0 }) }
    }

    fn set_canonical<'a>(
        &'a mut self,
        _b: &'a Block,
    ) -> impl Future<Output = Result<(), ExecError>> + Send + 'a {
        async move { Ok(()) }
    }

    fn seal<'a>(
        &'a mut self,
        _ub: &'a UnsealedBlock,
    ) -> impl Future<Output = Result<(), ExecError>> + Send + 'a {
        async move { Ok(()) }
    }

    fn get_block<'a>(
        &'a self,
        _hash: B256,
        _number: BlockNumber,
    ) -> impl Future<Output = Result<Block, ExecError>> + Send + 'a {
        async move { Ok(Block::default()) }
    }

    fn reset(&mut self) {}
}
