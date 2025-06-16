use alloy_primitives::{Address, Bytes};
use jsonrpsee::proc_macros::rpc;
use serde::{Deserialize, Serialize};

use crate::communication::messages::RpcResult;

pub const FRAG_COMMITMENT_TYPE: u64 = 7;

/// A CommitmentRequest message created by a user
#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all="camelCase")]
pub struct CommitmentRequest {
    pub commitment_type: u64,
    pub payload: Bytes,
    pub slasher: Address
}

/// A Commitment message responding to a CommitmentRequest
#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all="camelCase")]
pub struct Commitment {
    pub commitment_type: u64,
    pub payload: Bytes,
    pub request_hash: u64,
    pub slasher: Address
}

/// A signed Commitment binding to a CommitmentRequest
#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all="camelCase")]
pub struct SignedCommitment {
    pub commitment: Commitment,
    pub signature: Bytes,
}

/// Specifies which commitments can be made for a specific chain
#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all="camelCase")]
pub struct Offering {
    pub chain_id: u64,
    pub commitment_types: Vec<u64>,
}

/// Information about a Gateway's offerings at a specific slot
#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all="camelCase")]
pub struct SlotInfo {
    pub slot: u64,
    pub offerings: Vec<Offering>,
}

/// Response containing multiple SlotInfo
#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all="camelCase")]
pub struct SlotInfoResponse {
    pub slots: Vec<SlotInfo>,
}

/// Fee information for a specific commitment request
#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all="camelCase")]
pub struct FeeInfo {
    pub payload: Bytes,
    pub commitment_type: u64,
}

#[rpc(client, server, namespace = "gateway")]
pub trait FabricGatewayApi {
    /// Request a new SignedCommitment
    #[method(name = "commitment")]
    async fn post_commitment(&self, commitment_request: CommitmentRequest) -> RpcResult<SignedCommitment>;

    /// Request an existing SignedCommitment by request hash
    #[method(name = "getCommitment")]
    async fn get_commitment(&self, request_hash: u64) -> RpcResult<SignedCommitment>;

    /// Get Gateway information for upcoming slots
    #[method(name = "slots")]
    async fn get_slots(&self) -> RpcResult<SlotInfoResponse>;

    /// Get commitment fee information
    #[method(name = "fee")]
    async fn get_fee_info(&self, commitment_request: CommitmentRequest) -> RpcResult<FeeInfo>;
}