use std::sync::Arc;

use alloy_primitives::{Address, Bytes, B256, U256};
use alloy_rpc_types::{BlockId, BlockNumberOrTag};
use bop_common::{
    api::{EthApiClient, EthApiServer, OpRpcBlock},
    communication::messages::RpcResult,
    db::DatabaseRead,
    transaction::Transaction,
};
use jsonrpsee::core::async_trait;
use op_alloy_rpc_types::OpTransactionReceipt;
use tracing::{warn, Level, trace};

use crate::RpcServer;

/// Note: this is a temporary RPC implementation that only serves the lastest state from the sequencer.
/// It doesn't adhere to the specific block number or hash requests.
/// This will ultimately be replaced by the RPC server in the EL when the full Frag handling is implemented.
#[async_trait]
impl<D: DatabaseRead> EthApiServer for RpcServer<D> {
    #[tracing::instrument(skip_all, err, ret(level = Level::TRACE))]
    async fn send_raw_transaction(&self, bytes: Bytes) -> RpcResult<B256> {
        trace!(?bytes, "new request");

        let tx = Arc::new(Transaction::decode(bytes)?);
        let hash = tx.tx_hash();
        let _ = self.new_order_tx.send(tx.into());

        Ok(hash)
    }

    // STORE

    #[tracing::instrument(skip_all, err, ret(level = Level::TRACE))]
    async fn transaction_receipt(&self, hash: B256) -> RpcResult<Option<OpTransactionReceipt>> {
        trace!(%hash, "new request");

        let receipt = match self.shared_state.get_receipt(&hash) {
            Some(receipt) => Some(receipt),
            None => self.fallback.transaction_receipt(hash).await?,
        };

        Ok(receipt)
    }

    #[tracing::instrument(skip_all, err, ret(level = Level::TRACE))]
    async fn block_by_number(&self, number: BlockNumberOrTag, full: bool) -> RpcResult<Option<OpRpcBlock>> {
        trace!(%number, full, "new request");

        match self.shared_state.get_latest_block(full) {
            Some(block) => Ok(Some(block)),
            None => {
                Ok(self.fallback.block_by_number(number, full).await?)
            }
        }
    }

    #[tracing::instrument(skip_all, err, ret(level = Level::TRACE))]
    async fn block_by_hash(&self, hash: B256, full: bool) -> RpcResult<Option<OpRpcBlock>> {
        trace!(%hash, full, "new request");

        match self.shared_state.get_latest_block(full) {
            Some(block) => Ok(Some(block)),
            None => {
                Ok(self.fallback.block_by_hash(hash, full).await?)
            }
        }
    }

    #[tracing::instrument(skip_all, err, ret(level = Level::TRACE))]
    async fn block_number(&self) -> RpcResult<U256> {
        trace!("new request");

        let bn = match self.shared_state.get_latest_block_number() {
            Some(bn) => U256::from(bn),
            None => match self.shared_state.as_ref().head_block_number() {
                Ok(bn) => U256::from(bn),
                Err(err) => {
                    warn!(%err, "failed db fetch");
                    self.fallback.block_number().await?
                }
            },
        };

        Ok(bn)
    }

    #[tracing::instrument(skip_all, err, ret(level = Level::TRACE))]
    async fn transaction_count(&self, address: Address, block_number: Option<BlockId>) -> RpcResult<U256> {
        trace!(%address, ?block_number, "new request");

        let nonce = match self.shared_state.as_ref().get_nonce(address) {
            Ok(nonce) => U256::from(nonce),
            Err(err) => {
                warn!(%err, "failed db fetch");
                self.fallback.transaction_count(address, block_number).await?
            }
        };

        Ok(nonce)
    }

    #[tracing::instrument(skip_all, err, ret(level = Level::TRACE))]
    async fn balance(&self, address: Address, block_number: Option<BlockId>) -> RpcResult<U256> {
        trace!(%address, ?block_number, "new request");

        let balance = match self.shared_state.as_ref().get_balance(address) {
            Ok(balance) => U256::from(balance),
            Err(err) => {
                warn!(%err, "failed db fetch");
                self.fallback.balance(address, block_number).await?
            }
        };

        Ok(balance)
    }
}
