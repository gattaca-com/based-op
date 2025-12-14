use std::time::Duration;

use alloy_eips::Decodable2718;
use alloy_primitives::B256;
use bop_common::{
    communication::messages::{RpcError, RpcResult},
    fabric::{Commitment, CommitmentRequest, FabricGatewayApiServer, FeeInfo, SignedCommitment, SlotInfoResponse},
    order::Order,
    p2p::VersionedMessage,
    telemetry::TelemetryUpdate,
    transaction::Transaction,
};
use jsonrpsee::core::async_trait;
use op_alloy_consensus::OpTxEnvelope;
use tokio::time::timeout;
use tracing::Level;
use tree_hash::TreeHash;

use crate::RpcServer;

const COMMITMENT_TIMEOUT: Duration = Duration::from_secs(1);

#[async_trait]
impl FabricGatewayApiServer for RpcServer {
    #[tracing::instrument(skip_all, err, ret(level = Level::TRACE))]
    async fn post_commitment(&self, commitment: CommitmentRequest) -> RpcResult<SignedCommitment> {
        let request_hash = commitment.tree_hash_root();
        let tx = Transaction::decode(commitment.payload.to_vec().into())?;
        let tx_hash = tx.tx_hash();

        let mut receiver = self.frag_receiver_spawner.subscribe();

        // Send the transaction to the sequencer
        TelemetryUpdate::send_ref(tx.uuid, tx.to_ingested_telemetry(), &self.telemetry_producer);
        let order = Order::from(tx);
        let _ = self.new_order_tx.send(order.into());

        // Wait for the transaction to be committed
        let commitment_future = async {
            while let Ok(msg) = receiver.recv().await {
                tracing::info!("got {msg:?}");
                let VersionedMessage::FragV0(frag) = &msg.message else {
                    continue;
                };
                if frag
                    .txs
                    .iter()
                    .filter_map(|t| OpTxEnvelope::decode_2718(&mut t.as_ref()).ok())
                    .any(|t| *t.hash() == tx_hash)
                {
                    let commitment = Commitment {
                        commitment_type: commitment.commitment_type,
                        payload: serde_json::to_vec(&msg)?.into(),
                        request_hash,
                        slasher: commitment.slasher,
                    };
                    return Ok(SignedCommitment { commitment, signature: msg.signature });
                }
            }
            Err(RpcError::BroadcastChannelClosed)
        };

        timeout(COMMITMENT_TIMEOUT, commitment_future)
            .await
            .map_err(|_| RpcError::NoCommitmentForRequest(request_hash))?
    }

    #[tracing::instrument(skip_all, err, ret(level = Level::TRACE))]
    async fn get_commitment(&self, request_hash: B256) -> RpcResult<SignedCommitment> {
        // TODO: fetch from datastore
        Err(RpcError::NoCommitmentForRequest(request_hash))
    }

    #[tracing::instrument(skip_all, err, ret(level = Level::TRACE))]
    async fn get_slots(&self) -> RpcResult<SlotInfoResponse> {
        // Not implemented for L2s
        Ok(SlotInfoResponse { slots: vec![] })
    }

    #[tracing::instrument(skip_all, err, ret(level = Level::TRACE))]
    async fn get_fee_info(&self, _commitment_request: CommitmentRequest) -> RpcResult<FeeInfo> {
        // No additional fees in the gateway
        return Ok(FeeInfo::default());
    }
}
