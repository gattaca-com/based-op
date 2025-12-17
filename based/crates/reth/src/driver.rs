use std::time::Instant;
use alloy_primitives::B256;
use alloy_rpc_types::Block;
use anyhow::Context;
use tokio::sync::{mpsc, oneshot};
use tracing::{error, info};
use thiserror::Error;

use bop_common::p2p::{EnvV0, FragV0, SealV0};

use crate::exec::{apply_exec_output, ExecError, UnsealedExecutor};
use crate::unsealed_block::{UnsealedBlock, UnsealedBlockError};

#[derive(Debug, Clone, Copy)]
pub enum FragStatus {
    Valid,
    Invalid,
}

#[derive(Clone)]
pub struct Driver {
    tx: mpsc::Sender<Cmd>,
}

#[derive(Debug, Error)]
pub enum DriverError {
    #[error("failed to send command to driver task (task not running)")]
    DriverGone,

    #[error("failed to receive response from driver task (response dropped)")]
    ResponseDropped,

    #[error("driver not initialized, call env_v0 first")]
    NotInitialized,

    #[error("cannot open a new unsealed block while there's one already in progress (current={current}, incoming={incoming})")]
    UnsealedBlockInProgress {
        current: u64,
        incoming: u64,
    },

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

impl From<mpsc::error::SendError<Cmd>> for DriverError {
    fn from(_: mpsc::error::SendError<Cmd>) -> Self {
        DriverError::DriverGone
    }
}

impl From<oneshot::error::RecvError> for DriverError {
    fn from(_: oneshot::error::RecvError) -> Self {
        DriverError::ResponseDropped
    }
}

type Resp<T> = oneshot::Sender<Reply<T>>;

#[derive(Debug)]
enum Reply<T> {
    Ok(T),
    Err(DriverError),
}

impl<T> Reply<T> {
    fn into_result(self) -> Result<T, DriverError> {
        match self {
            Reply::Ok(v) => Ok(v),
            Reply::Err(e) => Err(e),
        }
    }
}

fn respond<T>(resp: Resp<T>, res: Result<T, DriverError>) {
    let _ = resp.send(match res {
        Ok(v) => Reply::Ok(v),
        Err(e) => Reply::Err(e),
    });
}

enum Cmd {
    EnvV0 { env: EnvV0, resp: Resp<()> },
    NewFragV0 { frag: FragV0, resp: Resp<FragStatus> },
    SealFragV0 { seal: SealV0, resp: Resp<()> },
    ForkchoiceUpdated { new_block_number: u64, resp: Resp<()> },
    GetHeaderView { resp: Resp<HeaderView> },
}

#[derive(Debug, Clone)]
pub struct HeaderView {
    pub enabled: bool,
    pub header: Option<alloy_consensus::Header>,
}

pub struct DriverInner<E: UnsealedExecutor> {
    pub enabled_unsealed_as_latest: bool,

    pub current_unsealed_block: Option<UnsealedBlock>,
    pub exec: E,

    pub fcu_count_since_unseal_reset: usize,
}

impl Driver {
    pub fn spawn<E: UnsealedExecutor + 'static + std::marker::Sync>(inner: DriverInner<E>) -> Self {
        let (tx, mut rx) = mpsc::channel::<Cmd>(256);

        tokio::spawn(async move {
            let mut inner = inner;

            while let Some(cmd) = rx.recv().await {
                match cmd {
                    Cmd::EnvV0 { env, resp } => {
                        respond(resp, inner.handle_env_v0(env).await);
                    }
                    Cmd::NewFragV0 { frag, resp } => {
                        respond(resp, inner.handle_new_frag_v0(frag).await);
                    }
                    Cmd::SealFragV0 { seal, resp } => {
                        respond(resp, inner.handle_seal_frag_v0(seal).await);
                    }
                    Cmd::ForkchoiceUpdated {
                        new_block_number,
                        resp,
                    } => {
                        respond(resp, inner.handle_forkchoice_updated(new_block_number).await);
                    }
                    Cmd::GetHeaderView { resp } => {
                        let _ = resp.send(Reply::Ok(inner.get_header_view()));
                    }
                }
            }
        });

        Self { tx }
    }

    pub async fn env_v0(&self, env: EnvV0) -> Result<(), DriverError> {
        let (resp_tx, resp_rx) = oneshot::channel();
        self.tx.send(Cmd::EnvV0 { env, resp: resp_tx }).await?;
        resp_rx.await?.into_result()
    }

    pub async fn new_frag_v0(&self, frag: FragV0) -> Result<FragStatus, DriverError> {
        let (resp_tx, resp_rx) = oneshot::channel();
        self.tx
            .send(Cmd::NewFragV0 { frag, resp: resp_tx })
            .await?;
        resp_rx.await?.into_result()
    }

    pub async fn seal_frag_v0(&self, seal_v0: SealV0) -> Result<(), DriverError> {
        let (resp_tx, resp_rx) = oneshot::channel();
        self.tx.send(Cmd::SealFragV0 { resp: resp_tx, seal: seal_v0 }).await?;
        resp_rx.await?.into_result()
    }

    pub async fn forkchoice_updated(&self, new_block_number: u64) -> Result<(), DriverError> {
        let (resp_tx, resp_rx) = oneshot::channel();
        self.tx
            .send(Cmd::ForkchoiceUpdated {
                new_block_number,
                resp: resp_tx,
            })
            .await?;
        resp_rx.await?.into_result()
    }

    pub async fn header_view(&self) -> HeaderView {
        let (resp_tx, resp_rx) = oneshot::channel();

        if self.tx.send(Cmd::GetHeaderView { resp: resp_tx }).await.is_err() {
            return HeaderView {
                enabled: false,
                header: None,
            };
        }

        match resp_rx.await {
            Ok(reply) => reply.into_result().unwrap_or(HeaderView {
                enabled: false,
                header: None,
            }),
            Err(_) => HeaderView {
                enabled: false,
                header: None,
            },
        }
    }
}

impl<E: UnsealedExecutor> DriverInner<E> {
    fn reset_current_unsealed_block(&mut self) {
        self.current_unsealed_block = None;
        self.exec.reset();
        self.fcu_count_since_unseal_reset = 0;
    }

    async fn handle_env_v0(&mut self, env: EnvV0) -> Result<(), DriverError> {
        info!(block = env.number, "envV0 received");

        if let Some(current) = self.current_unsealed_block.as_ref() {
            let current_num = current.env.number;

            if current_num >= env.number {
                return Err(DriverError::UnsealedBlockInProgress {
                    current: current_num,
                    incoming: env.number,
                });
            }

            info!(old = current_num, new = env.number, "env advanced, resetting");
            self.reset_current_unsealed_block()
        }

        self.exec.ensure_env(&env).context("exec.ensure_env")?;
        self.current_unsealed_block = Some(UnsealedBlock::new(env));
        self.fcu_count_since_unseal_reset = 0;
        Ok(())
    }

    async fn handle_forkchoice_updated(&mut self, new_block_number: u64) -> Result<(), DriverError> {
        self.fcu_count_since_unseal_reset += 1;

        let Some(ub) = self.current_unsealed_block.as_ref() else {
            return Ok(());
        };

        if ub.env.number != new_block_number {
            info!(
                old = ub.env.number,
                new = new_block_number,
                "forkchoiceUpdated block mismatch: resetting unsealed"
            );
            self.reset_current_unsealed_block();
        }

        Ok(())
    }

    async fn handle_new_frag_v0(&mut self, frag: FragV0) -> Result<FragStatus, DriverError> {
        let start = Instant::now();

        let Some(ub) = self.current_unsealed_block.as_mut() else {
            return Err(DriverError::NotInitialized);
        };

        info!(for_block = frag.block_number, current = ub.env.number, "new frag received");

        if frag.block_number < ub.env.number {
            info!(
                frag_block = frag.block_number,
                env_number = ub.env.number,
                "stale frag (older block), ignoring"
            );
            return Ok(FragStatus::Valid);
        }

        if let Err(e) = ub.validate_new_frag(&frag) {
            error!(error = %e, "frag invalid, discarding unsealed block");
            self.reset_current_unsealed_block();
            return Err(DriverError::from(e));
        }

        let out = match self.exec.execute_frag(ub, &frag).await {
            Ok(out) => out,
            Err(e) => {
                error!(error = %e, "execution failed, discarding unsealed block");
                self.reset_current_unsealed_block();
                return Err(DriverError::from(e));
            }
        };

        apply_exec_output(ub, out);
        ub.accept_frag(frag);

        info!(elapsed_ms = start.elapsed().as_millis(), "frag inserted + executed");

        if ub.last_frag().is_some_and(|f| f.is_last) {
            info!("last frag received, pre-sealing block");
            if let Err(e) = self.exec.seal(ub).await {
                error!(error = %e, "seal failed, discarding unsealed block");
                self.reset_current_unsealed_block();
                return Err(DriverError::from(e));
            }
        }

        Ok(FragStatus::Valid)
    }

    async fn handle_seal_frag_v0(&mut self, seal: SealV0) -> Result<(), DriverError> {
        let start = Instant::now();
        let Some(ub) = self.current_unsealed_block.as_ref() else {
            return Err(DriverError::NotInitialized);
        };

        if ub.env.number > seal.block_number {
            info!(ub = ub.env.number, seal = seal.block_number, "stale seal, dropping");
            return Ok(());
        }

        let presealed_block = self.exec.get_block(seal.block_hash, seal.block_number).await?;
        self.validate_seal_frag_v0(&presealed_block, &ub, seal).await?;

        self.exec.set_canonical(&presealed_block).await.context("sealFragV0")?;

        self.reset_current_unsealed_block();

        info!(elapsed_ms = start.elapsed().as_millis(), "block sealed");
        Ok(())
    }

    async fn validate_seal_frag_v0(&self, presealed_block: &Block, ub: &UnsealedBlock, seal: SealV0) -> Result<(), ValidateSealError> {
        let expected_block_hash: B256 = presealed_block.header.hash.into();
        if expected_block_hash != seal.block_hash {
            return Err(ValidateSealError::BlockHashMismatch {
                expected: expected_block_hash,
                got: seal.block_hash,
            });
        }

        let expected_parent_hash = presealed_block.header.parent_hash;
        if expected_parent_hash != seal.parent_hash {
            return Err(ValidateSealError::ParentHashMismatch {
                expected: expected_parent_hash,
                got: seal.parent_hash,
            });
        }

        let expected_state_root = presealed_block.header.state_root;
        if expected_state_root != seal.state_root {
            return Err(ValidateSealError::StateRootMismatch {
                expected: expected_state_root,
                got: seal.state_root,
            });
        }

        let expected_tx_root = presealed_block.header.transactions_root;
        if expected_tx_root != seal.transactions_root {
            return Err(ValidateSealError::TransactionsRootMismatch {
                expected: expected_tx_root,
                got: seal.transactions_root,
            });
        }

        let expected_receipts_root = presealed_block.header.receipts_root;
        if expected_receipts_root != seal.receipts_root {
            return Err(ValidateSealError::ReceiptsRootMismatch {
                expected: expected_receipts_root,
                got: seal.receipts_root,
            });
        }

        let expected_gas_used = presealed_block.header.gas_used;
        if expected_gas_used != seal.gas_used {
            return Err(ValidateSealError::GasUsedMismatch {
                expected: expected_gas_used,
                got: seal.gas_used,
            });
        }

        let expected_gas_limit = presealed_block.header.gas_limit;
        if expected_gas_limit != seal.gas_limit {
            return Err(ValidateSealError::GasLimitMismatch {
                expected: expected_gas_limit,
                got: seal.gas_limit,
            });
        }

        let expected_total_frags = ub.frags.len() as u64;
        if expected_total_frags != seal.total_frags {
            return Err(ValidateSealError::TotalFragsMismatch {
                expected: expected_total_frags,
                got: seal.total_frags,
            });
        }
        Ok(())
    }

    fn get_header_view(&self) -> HeaderView {
        if !self.enabled_unsealed_as_latest {
            return HeaderView {
                enabled: false,
                header: None,
            };
        }
        let header = self
            .current_unsealed_block
            .as_ref()
            .map(|ub| ub.temp_header());
        HeaderView { enabled: true, header }
    }
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