use std::{sync::Arc, time::Instant};

use alloy_consensus::Header;
use alloy_primitives::B256;
use alloy_rpc_types::Block;
use arc_swap::ArcSwapOption;
use bop_common::{
    p2p::{EnvV0, FragV0, SealV0},
    typedefs::OpBlock,
};
use reth_chainspec::{ChainSpecProvider, EthChainSpec};
use reth_optimism_chainspec::OpHardforks;
use reth_storage_api::{BlockReaderIdExt, StateProviderFactory};
use tokio::sync::{mpsc, oneshot};
use tracing::{error, info};

use crate::{
    error::{DriverError, ValidateSealError},
    exec::{StateExecutor, UnsealedExecutor},
    unsealed_block::UnsealedBlock,
};

/// Result of submitting a frag to the driver.
#[derive(Debug, Clone, Copy)]
pub enum FragStatus {
    Valid,
    Invalid,
}

/// Actor handle for sending unsealed-block commands to the driver task.
#[derive(Clone)]
pub struct Driver {
    tx: mpsc::Sender<Cmd>,
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
    ForkchoiceUpdated { block: OpBlock, resp: Resp<()> },
    GetHeaderView { resp: Resp<HeaderView> },
}

#[derive(Debug, Clone)]
pub struct HeaderView {
    pub enabled: bool,
    pub header: Option<Header>,
}

/// Single-threaded state owned by the driver task (unsealed block + executor + counters).
/// Essentially should be implemented using based-op-reth
#[derive(Debug)]
pub struct DriverInner<E: UnsealedExecutor> {
    pub enabled_unsealed_as_latest: bool,
    pub current_unsealed_block: Arc<ArcSwapOption<UnsealedBlock>>,
    pub exec: E,
    pub fcu_count_since_unseal_reset: usize,
}

impl Driver {
    pub fn new<Client>(unsealed_as_latest: bool, client: Client) -> Self
    where
        Client: StateProviderFactory
            + ChainSpecProvider<ChainSpec: EthChainSpec<Header = Header> + OpHardforks>
            + BlockReaderIdExt<Header = Header>
            + Clone
            + 'static,
    {
        let executor = StateExecutor::new(client);
        let current_unsealed_block = executor.shared_unsealed_block();

        Self::spawn(DriverInner {
            enabled_unsealed_as_latest: unsealed_as_latest,
            current_unsealed_block,
            exec: executor,
            fcu_count_since_unseal_reset: 0,
        })
    }

    /// Spawns the driver actor task and returns a handle used to send commands to it.
    pub fn spawn<E: UnsealedExecutor + 'static>(inner: DriverInner<E>) -> Self {
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
                    Cmd::ForkchoiceUpdated { block, resp } => {
                        respond(resp, inner.handle_forkchoice_updated(block).await);
                    }
                    Cmd::GetHeaderView { resp } => {
                        let _ = resp.send(Reply::Ok(inner.get_header_view()));
                    }
                }
            }
        });

        Self { tx }
    }

    /// Starts a new unsealed block execution context for the given environment.
    pub async fn env_v0(&self, env: EnvV0) -> Result<(), DriverError> {
        let (resp_tx, resp_rx) = oneshot::channel();
        self.tx.send(Cmd::EnvV0 { env, resp: resp_tx }).await?;
        resp_rx.await?.into_result()
    }

    /// Executes and records a fragment against the current unsealed block.
    pub async fn new_frag_v0(&self, frag: FragV0) -> Result<FragStatus, DriverError> {
        let (resp_tx, resp_rx) = oneshot::channel();
        self.tx.send(Cmd::NewFragV0 { frag, resp: resp_tx }).await?;
        resp_rx.await?.into_result()
    }

    /// Validates and finalizes the current unsealed block using the provided seal.
    pub async fn seal_frag_v0(&self, seal_v0: SealV0) -> Result<(), DriverError> {
        let (resp_tx, resp_rx) = oneshot::channel();
        self.tx.send(Cmd::SealFragV0 { resp: resp_tx, seal: seal_v0 }).await?;
        resp_rx.await?.into_result()
    }

    /// Notifies the driver about a forkchoice update and resets state on mismatch.
    pub async fn forkchoice_updated(&self, block: OpBlock) -> Result<(), DriverError> {
        let (resp_tx, resp_rx) = oneshot::channel();
        self.tx.send(Cmd::ForkchoiceUpdated { block, resp: resp_tx }).await?;
        resp_rx.await?.into_result()
    }

    /// Returns a best-effort view of the current unsealed header used as "latest" when enabled.
    pub async fn header_view(&self) -> HeaderView {
        let (resp_tx, resp_rx) = oneshot::channel();

        if self.tx.send(Cmd::GetHeaderView { resp: resp_tx }).await.is_err() {
            return HeaderView { enabled: false, header: None };
        }

        match resp_rx.await {
            Ok(reply) => reply.into_result().unwrap_or(HeaderView { enabled: false, header: None }),
            Err(_) => HeaderView { enabled: false, header: None },
        }
    }
}

impl<E: UnsealedExecutor> DriverInner<E> {
    fn reset_current_unsealed_block(&mut self) {
        self.exec.reset();
        self.fcu_count_since_unseal_reset = 0;
    }

    async fn handle_env_v0(&mut self, env: EnvV0) -> Result<(), DriverError> {
        info!(block = env.number, "envV0 received");

        if let Some(current) = self.current_unsealed_block.load_full().as_ref() {
            let current_num = current.env.number;

            if current_num >= env.number {
                return Err(DriverError::UnsealedBlockInProgress { current: current_num, incoming: env.number });
            }

            info!(old = current_num, new = env.number, "env advanced, resetting");
            self.reset_current_unsealed_block();
        }

        self.exec.ensure_env(&env)?; // this should update current_unsealed_block too because shared arc
        self.fcu_count_since_unseal_reset = 0;
        Ok(())
    }

    async fn handle_forkchoice_updated(&mut self, block: OpBlock) -> Result<(), DriverError> {
        self.fcu_count_since_unseal_reset += 1;

        let Some(ub) = self.current_unsealed_block.load_full() else {
            return Ok(());
        };

        let new_block_number = block.header.number;

        // TODO: Check hash etc. Commit state if needed
        if ub.env.number != new_block_number {
            info!(old = ub.env.number, new = new_block_number, "forkchoiceUpdated block mismatch: resetting unsealed");
            self.reset_current_unsealed_block();
        }

        Ok(())
    }

    async fn handle_new_frag_v0(&mut self, frag: FragV0) -> Result<FragStatus, DriverError> {
        let start = Instant::now();

        let Some(ub) = self.current_unsealed_block.load_full() else {
            return Err(DriverError::NotInitialized);
        };

        info!(for_block = frag.block_number, current = ub.env.number, "new frag received");

        if frag.block_number < ub.env.number {
            info!(frag_block = frag.block_number, env_number = ub.env.number, "stale frag (older block), ignoring");
            return Ok(FragStatus::Valid);
        }

        if let Err(e) = ub.validate_new_frag(&frag) {
            error!(error = %e, "frag invalid, discarding unsealed block");
            self.reset_current_unsealed_block();
            return Err(DriverError::from(e));
        }

        match self.exec.execute_frag(&frag).await {
            Ok(()) => (),
            Err(e) => {
                error!(error = %e, "execution failed, discarding unsealed block");
                self.reset_current_unsealed_block();
                return Err(DriverError::from(e));
            }
        };

        info!(elapsed_ms = start.elapsed().as_millis(), "frag inserted + executed");

        if ub.last_frag().is_some_and(|f| f.is_last) {
            info!("last frag received, pre-sealing block");
            if let Err(e) = self.exec.seal().await {
                error!(error = %e, "seal failed, discarding unsealed block");
                self.reset_current_unsealed_block();
                return Err(DriverError::from(e));
            }
        }

        Ok(FragStatus::Valid)
    }

    async fn handle_seal_frag_v0(&mut self, seal: SealV0) -> Result<(), DriverError> {
        let start = Instant::now();
        let Some(ub) = self.current_unsealed_block.load_full() else {
            return Err(DriverError::NotInitialized);
        };

        if ub.env.number > seal.block_number {
            info!(ub = ub.env.number, seal = seal.block_number, "stale seal, dropping");
            return Ok(());
        }

        let presealed_block = self.exec.get_block(seal.block_hash, seal.block_number).await;

        let presealed_block = match presealed_block {
            Ok(b) => b,
            Err(e) => {
                self.reset_current_unsealed_block();
                return Err(DriverError::from(e));
            }
        };

        self.validate_seal_frag_v0(&presealed_block, ub.as_ref(), &seal)?;

        self.exec.set_canonical(&presealed_block).await?;

        self.reset_current_unsealed_block();

        info!(elapsed_ms = start.elapsed().as_millis(), "block sealed");
        Ok(())
    }

    fn validate_seal_frag_v0(
        &self,
        presealed_block: &Block,
        ub: &UnsealedBlock,
        seal: &SealV0,
    ) -> Result<(), ValidateSealError> {
        let expected_block_hash: B256 = presealed_block.header.hash.into();
        if expected_block_hash != seal.block_hash {
            return Err(ValidateSealError::BlockHashMismatch { expected: expected_block_hash, got: seal.block_hash });
        }

        let expected_parent_hash = presealed_block.header.parent_hash;
        if expected_parent_hash != seal.parent_hash {
            return Err(ValidateSealError::ParentHashMismatch { expected: expected_parent_hash, got: seal.parent_hash });
        }

        let expected_state_root = presealed_block.header.state_root;
        if expected_state_root != seal.state_root {
            return Err(ValidateSealError::StateRootMismatch { expected: expected_state_root, got: seal.state_root });
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
            return Err(ValidateSealError::GasUsedMismatch { expected: expected_gas_used, got: seal.gas_used });
        }

        let expected_gas_limit = presealed_block.header.gas_limit;
        if expected_gas_limit != seal.gas_limit {
            return Err(ValidateSealError::GasLimitMismatch { expected: expected_gas_limit, got: seal.gas_limit });
        }

        let expected_total_frags = ub.frags.len() as u64;
        if expected_total_frags != seal.total_frags {
            return Err(ValidateSealError::TotalFragsMismatch { expected: expected_total_frags, got: seal.total_frags });
        }

        Ok(())
    }

    fn get_header_view(&self) -> HeaderView {
        if !self.enabled_unsealed_as_latest {
            return HeaderView { enabled: false, header: None };
        }
        let header = match self.current_unsealed_block.load_full() {
            Some(ub) => Some(ub.get_header()),
            None => None,
        };
        HeaderView { enabled: true, header }
    }
}
