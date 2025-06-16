use std::{sync::Arc, time::Duration};

use alloy_eips::Decodable2718;
use bop_common::{
    communication::messages::RpcResult, fabric::{Commitment, CommitmentRequest, FabricGatewayApiServer, FeeInfo, SignedCommitment, SlotInfoResponse}, p2p::VersionedMessage, telemetry::TelemetryUpdate, transaction::Transaction
};
use jsonrpsee::{core::async_trait};
use op_alloy_consensus::OpTxEnvelope;
use tokio::time::timeout;
use tracing::Level;

use crate::RpcServer;

const COMMITMENT_TIMEOUT: Duration = Duration::from_secs(1);

#[async_trait]
impl FabricGatewayApiServer for RpcServer {
    #[tracing::instrument(skip_all, err, ret(level = Level::TRACE))]
    async fn post_commitment(&self, commitment: CommitmentRequest) -> RpcResult<SignedCommitment> {
        // Send the transaction to the sequencer
        let tx = Arc::new(Transaction::decode(commitment.payload.clone())?);
        TelemetryUpdate::send_ref(tx.uuid, tx.to_ingested_telemetry(), &self.telemetry_producer);
        let hash = tx.tx_hash();
        let _ = self.new_order_tx.send(tx.into());

        // Wait for the transaction to be committed
        let mut receiver = self.frag_receiver_spawner.subscribe();

        let commitment_future = async {
            loop {
                if let Ok(msg) = receiver.recv().await {
                    let VersionedMessage::FragV0(frag) = &msg.message else {
                        continue;
                    };
                    if frag
                        .txs
                        .iter()
                        .filter_map(|t| OpTxEnvelope::decode_2718(&mut t.as_ref()).ok())
                        .any(|t| *t.hash() == hash)
                    {
                        let commitment = Commitment {
                            commitment_type: commitment.commitment_type,
                            payload: commitment.payload,
                            request_hash: u64::from_be_bytes(hash[..8].try_into().unwrap()),
                            slasher: commitment.slasher,
                        };
                        return Ok(SignedCommitment { commitment, signature: msg.signature });
                    }
                }
            }
        };

        let commitment_result = timeout(COMMITMENT_TIMEOUT, commitment_future).await?;
        commitment_result
    }

    #[tracing::instrument(skip_all, err, ret(level = Level::TRACE))]
    async fn get_commitment(&self, _request_hash: u64) -> RpcResult<SignedCommitment> {
        todo!()
    }

    #[tracing::instrument(skip_all, err, ret(level = Level::TRACE))]
    async fn get_slots(&self) -> RpcResult<SlotInfoResponse> {
        todo!()
    }

    #[tracing::instrument(skip_all, err, ret(level = Level::TRACE))]
    async fn get_fee_info(&self, _commitment_request: CommitmentRequest) -> RpcResult<FeeInfo> {
        // No additional fees in the gateway
        return Ok(FeeInfo::default());
    }
}