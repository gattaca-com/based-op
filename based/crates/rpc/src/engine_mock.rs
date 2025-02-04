use alloy_rpc_types::engine::{
    ExecutionPayloadEnvelopeV2, ExecutionPayloadV1, ExecutionPayloadV2, ExecutionPayloadV3, PayloadStatus,
};
use bop_common::{
    actor::Actor,
    communication::{messages::EngineApi, Sender, SpineConnections},
    db::BopDbRead,
};
use tokio::sync::oneshot;

pub struct MockEngineRpcServer {
    last_block_number: u64,
}

impl MockEngineRpcServer {
    pub fn new(last_block_number: u64) -> Self {
        Self { last_block_number }
    }
}

impl<Db: BopDbRead> Actor<Db> for MockEngineRpcServer {
    fn on_init(&mut self, connections: &mut SpineConnections<Db>) {
        let (tx, rx) = oneshot::channel();
        let payload = ExecutionPayloadV3 {
            payload_inner: ExecutionPayloadV2 {
                payload_inner: ExecutionPayloadV1 {
                    parent_hash: Default::default(),
                    fee_recipient: Default::default(),
                    state_root: Default::default(),
                    receipts_root: Default::default(),
                    logs_bloom: Default::default(),
                    prev_randao: Default::default(),
                    block_number: self.last_block_number,
                    gas_limit: Default::default(),
                    gas_used: Default::default(),
                    timestamp: Default::default(),
                    extra_data: Default::default(),
                    base_fee_per_gas: Default::default(),
                    block_hash: Default::default(),
                    transactions: Default::default(),
                },
                withdrawals: Default::default(),
            },
            blob_gas_used: Default::default(),
            excess_blob_gas: Default::default(),
        };
        connections.send(EngineApi::NewPayloadV3 {
            payload,
            versioned_hashes: Default::default(),
            parent_beacon_block_root: Default::default(),
            res_tx: tx,
        });
        let (tx, rx) = oneshot::channel();
        connections.send(EngineApi::ForkChoiceUpdatedV3 {
            fork_choice_state: Default::default(),
            payload_attributes: Default::default(),
            res_tx: tx,
        });
    }
}
