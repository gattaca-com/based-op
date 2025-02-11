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
use reth_optimism_primitives::OpBlock;
use tracing::{trace, warn, Level};

use crate::RpcServer;

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

        let block = match number {
            BlockNumberOrTag::Latest => self.shared_state.get_latest_block().map(|block| convert_block(block, full)),
            BlockNumberOrTag::Number(bn) => {
                self.shared_state.get_block_by_number(bn).map(|block| convert_block(block, full))
            }
            _ => None,
        };

        if block.is_none() {
            Ok(self.fallback.block_by_number(number, full).await?)
        } else {
            Ok(block)
        }
    }

    #[tracing::instrument(skip_all, err, ret(level = Level::TRACE))]
    async fn block_by_hash(&self, hash: B256, full: bool) -> RpcResult<Option<OpRpcBlock>> {
        trace!(%hash, full, "new request");

        let block = match self.shared_state.get_block_by_hash(hash) {
            Some(block) => Some(convert_block(block, full)),
            None => self.fallback.block_by_hash(hash, full).await?,
        };

        Ok(block)
    }

    #[tracing::instrument(skip_all, err, ret(level = Level::TRACE))]
    async fn block_number(&self) -> RpcResult<U256> {
        trace!("block number request");

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
        let is_latest = block_number.map(|bn| bn.is_latest()).unwrap_or(true);
        let nonce = if is_latest {
            match self.shared_state.as_ref().get_nonce(address) {
                Ok(nonce) => U256::from(nonce),
                Err(err) => {
                    warn!(%err, "failed db fetch");
                    self.fallback.transaction_count(address, block_number).await?
                }
            }
        } else {
            self.fallback.transaction_count(address, block_number).await?
        };

        Ok(nonce)
    }

    #[tracing::instrument(skip_all, err, ret(level = Level::TRACE))]
    async fn balance(&self, address: Address, block_number: Option<BlockId>) -> RpcResult<U256> {
        trace!(%address, ?block_number, "new request");

        let is_latest = block_number.map(|bn| bn.is_latest()).unwrap_or(true);
        let balance = if is_latest {
            match self.shared_state.as_ref().get_balance(address) {
                Ok(balance) => U256::from(balance),
                Err(err) => {
                    warn!(%err, "failed db fetch");
                    self.fallback.balance(address, block_number).await?
                }
            }
        } else {
            self.fallback.balance(address, block_number).await?
        };

        Ok(balance)
    }
}

fn convert_block(_block: OpBlock, _full: bool) -> OpRpcBlock {
    todo!()
}
