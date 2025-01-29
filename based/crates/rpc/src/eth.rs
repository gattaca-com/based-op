use std::{net::SocketAddr, sync::Arc};

use alloy_primitives::{Address, Bytes, B256, U256};
use alloy_rpc_types::{Block, BlockId, BlockNumberOrTag, TransactionReceipt};
use bop_common::{
    api::EthApiServer,
    communication::{messages::RpcResult, Sender, Spine},
    transaction::Transaction,
};
use bop_db::DB;
use jsonrpsee::{core::async_trait, server::ServerBuilder};
use tracing::{error, info, trace, Level};

pub struct EthRpcServer {
    new_order_tx: Sender<Arc<Transaction>>,
    db: DB,
}

impl EthRpcServer {
    pub fn new(spine: &Spine, db: DB) -> Self {
        Self { new_order_tx: spine.into(), db }
    }

    #[tracing::instrument(skip_all, name = "rpc_eth")]
    pub async fn run(self, addr: SocketAddr) {
        info!(%addr, "starting RPC server");

        let server = ServerBuilder::default().build(addr).await.expect("failed to create eth RPC server");
        let module = EthApiServer::into_rpc(self);
        let server_handle = server.start(module);

        server_handle.stopped().await;
        error!("server stopped");
    }
}

#[async_trait]
impl EthApiServer for EthRpcServer {
    #[tracing::instrument(skip_all, err, ret(level = Level::TRACE))]
    async fn send_raw_transaction(&self, bytes: Bytes) -> RpcResult<B256> {
        trace!(?bytes, "new request");

        let tx = Arc::new(Transaction::decode(bytes)?);
        let hash = tx.hash();
        let _ = self.new_order_tx.send(tx.into());

        Ok(hash)
    }

    // STORE

    #[tracing::instrument(skip_all, err, ret(level = Level::TRACE))]
    async fn transaction_receipt(&self, hash: B256) -> RpcResult<Option<TransactionReceipt>> {
        trace!(%hash, "new request");

        todo!()
    }

    #[tracing::instrument(skip_all, err, ret(level = Level::TRACE))]
    async fn block_by_number(&self, number: BlockNumberOrTag, full: bool) -> RpcResult<Option<Block>> {
        trace!(%number, full, "new request");

        todo!()
    }

    #[tracing::instrument(skip_all, err, ret(level = Level::TRACE))]
    async fn block_by_hash(&self, hash: B256, full: bool) -> RpcResult<Option<Block>> {
        trace!(%hash, full, "new request");

        todo!()
    }

    // DB

    #[tracing::instrument(skip_all, err, ret(level = Level::TRACE))]
    async fn block_number(&self) -> RpcResult<U256> {
        trace!("new request");

        todo!()
    }

    #[tracing::instrument(skip_all, err, ret(level = Level::TRACE))]
    async fn transaction_count(&self, address: Address, block_number: Option<BlockId>) -> RpcResult<U256> {
        trace!(%address, ?block_number, "new request");
        let nonce = self.db.get_nonce(address);

        Ok(U256::from(nonce))
    }

    #[tracing::instrument(skip_all, err, ret(level = Level::TRACE))]
    async fn balance(&self, address: Address, block_number: Option<BlockId>) -> RpcResult<U256> {
        trace!(%address, ?block_number, "new request");

        todo!()
    }
}
