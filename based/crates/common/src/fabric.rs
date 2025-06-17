use alloy_primitives::{Address, B256, Bytes};
use jsonrpsee::proc_macros::rpc;
use serde::{Deserialize, Serialize};
use ssz_types::{VariableList, typenum};
use tree_hash_derive::TreeHash;

use crate::communication::messages::RpcResult;

pub type MaxCommitmentRequestPayloadSize = typenum::U1048576; // 1MB.
pub type CommitmentRequestPayload = VariableList<u8, MaxCommitmentRequestPayloadSize>;

pub const FRAG_COMMITMENT_TYPE: u64 = 7;

/// A CommitmentRequest message created by a user
#[derive(Clone, Serialize, Deserialize, Debug, TreeHash)]
#[serde(rename_all = "camelCase")]
pub struct CommitmentRequest {
    pub commitment_type: u64,
    pub payload: CommitmentRequestPayload,
    pub slasher: Address,
}

/// A Commitment message responding to a CommitmentRequest
#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Commitment {
    pub commitment_type: u64,
    pub payload: Bytes,
    pub request_hash: B256,
    pub slasher: Address,
}

/// A signed Commitment binding to a CommitmentRequest
#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SignedCommitment {
    pub commitment: Commitment,
    pub signature: Bytes,
}

/// Specifies which commitments can be made for a specific chain
#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Offering {
    pub chain_id: u64,
    pub commitment_types: Vec<u64>,
}

/// Information about a Gateway's offerings at a specific slot
#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SlotInfo {
    pub slot: u64,
    pub offerings: Vec<Offering>,
}

/// Response containing multiple SlotInfo
#[derive(Clone, Serialize, Deserialize, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct SlotInfoResponse {
    pub slots: Vec<SlotInfo>,
}

/// Fee information for a specific commitment request
#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FeeInfo {
    pub payload: Bytes,
    pub commitment_type: u64,
}

impl Default for FeeInfo {
    fn default() -> Self {
        Self { payload: Bytes::new(), commitment_type: FRAG_COMMITMENT_TYPE }
    }
}

#[rpc(client, server, namespace = "gateway")]
pub trait FabricGatewayApi {
    /// Request a new SignedCommitment
    #[method(name = "commitment")]
    async fn post_commitment(&self, commitment_request: CommitmentRequest) -> RpcResult<SignedCommitment>;

    /// Request an existing SignedCommitment by request hash
    #[method(name = "getCommitment")]
    async fn get_commitment(&self, request_hash: B256) -> RpcResult<SignedCommitment>;

    /// Get Gateway information for upcoming slots
    #[method(name = "slots")]
    async fn get_slots(&self) -> RpcResult<SlotInfoResponse>;

    /// Get commitment fee information
    #[method(name = "fee")]
    async fn get_fee_info(&self, commitment_request: CommitmentRequest) -> RpcResult<FeeInfo>;
}
