use std::{net::SocketAddr, sync::Arc};

use alloy_primitives::{B256, Bytes};
use alloy_rpc_types::engine::JwtSecret;
use bop_common::{
    api::{CommitmentFabric, EngineApiServer, GatewayApiServer, MinimalEthApiServer, SignedCommitmentFabric}, communication::{
        messages::{EngineApi, RpcResult}, Producer, Sender, Spine
    }, config::GatewayArgs, db::DatabaseRead, p2p::FragV0, telemetry::{telemetry_queue, TelemetryUpdate}, time::Duration, transaction::Transaction
};
use jsonrpsee::{core::async_trait, server::ServerBuilder};
use reth_rpc_layer::{AuthLayer, JwtAuthValidator};
use tokio::runtime::Runtime;
use tracing::{Level, error, info, trace};

use crate::RpcServer;

#[async_trait]
impl GatewayApiServer for RpcServer {
    #[tracing::instrument(skip_all, err, ret(level = Level::TRACE))]
    async fn commitment(&self, commitment: CommitmentFabric) -> RpcResult<SignedCommitmentFabric> {

        let tx = Arc::new(Transaction::decode(commitment.payload)?);
        TelemetryUpdate::send_ref(tx.uuid, tx.to_ingested_telemetry(), &self.telemetry_producer);
        let hash = tx.tx_hash();
        let _ = self.new_order_tx.send(tx.into());

        

        Ok(hash)
    }
}
