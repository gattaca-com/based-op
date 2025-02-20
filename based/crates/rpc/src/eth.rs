use std::sync::Arc;

use alloy_primitives::{Bytes, B256};
use bop_common::{
    api::MinimalEthApiServer, communication::messages::RpcResult, db::DatabaseRead, transaction::Transaction,
};
use jsonrpsee::core::async_trait;
use tracing::{trace, Level};

use crate::RpcServer;

/// Note: this is a temporary RPC implementation that only serves the lastest state from the sequencer.
/// It doesn't adhere to the specific block number or hash requests.
/// This will ultimately be replaced by the RPC server in the EL when the full Frag handling is implemented.
#[async_trait]
impl<D: DatabaseRead> MinimalEthApiServer for RpcServer<D> {
    #[tracing::instrument(skip_all, err, ret(level = Level::TRACE))]
    async fn send_raw_transaction(&self, bytes: Bytes) -> RpcResult<B256> {
        trace!(?bytes, "new request");

        let tx = Arc::new(Transaction::decode(bytes)?);
        let hash = tx.tx_hash();
        let _ = self.new_order_tx.send(tx.into());

        Ok(hash)
    }
}
