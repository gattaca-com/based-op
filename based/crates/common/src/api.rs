use alloy_primitives::{Address, Bytes, B256, U256};
use alloy_rpc_types::{
    engine::{ExecutionPayloadV3, ForkchoiceState, ForkchoiceUpdated, PayloadId, PayloadStatus},
    BlockId, BlockNumberOrTag,
};
use jsonrpsee::proc_macros::rpc;
use op_alloy_rpc_types::OpTransactionReceipt;
use op_alloy_rpc_types_engine::{OpExecutionPayloadEnvelopeV3, OpPayloadAttributes};
use reth_optimism_primitives::OpTransactionSigned;

use crate::communication::messages::RpcResult;

pub const CAPABILITIES: &[&str] =
    &["engine_forkchoiceUpdatedV3", "engine_getPayloadV3", "engine_newPayloadV3", "eth_sendRawTransaction"];

pub type OpRpcBlock = alloy_rpc_types::Block<OpTransactionSigned>;

/// The Engine API is used by the consensus layer to interact with the execution layer. Here we
/// implement a minimal subset of the API for the gateway to return blocks to the op-node
///
/// ref: https://github.com/ethereum/execution-apis/tree/main/src/engine
/// ref: https://specs.optimism.io/protocol/exec-engine.html#engine-api
///
/// NOTE: currently only v3 endpoints are supported
#[rpc(client, server, namespace = "engine")]
pub trait EngineApi {
    /// Used by the op-node to set which blocks are considered canonical.
    ///
    /// If payload attributes is set then block production for next block should start and a
    /// `PayloadId` is returned to be called in `get_payload`
    #[method(name = "forkchoiceUpdatedV3")]
    async fn fork_choice_updated_v3(
        &self,
        fork_choice_state: ForkchoiceState,
        payload_attributes: Option<OpPayloadAttributes>,
    ) -> RpcResult<ForkchoiceUpdated>;

    /// Used to validate an execution payload
    #[method(name = "newPayloadV3")]
    async fn new_payload_v3(
        &self,
        payload: ExecutionPayloadV3,
        versioned_hashes: Vec<B256>,
        parent_beacon_block_root: B256,
    ) -> RpcResult<PayloadStatus>;

    /// Used to fetch an execution payload from a previous `payload_id` set in `forkchoiceUpdatedV3`
    #[method(name = "getPayloadV3")]
    async fn get_payload_v3(&self, payload_id: PayloadId) -> RpcResult<OpExecutionPayloadEnvelopeV3>;
}

/// The Eth API is used to interact with the EL directly.
///
/// This is a temporary API that the gateway implements to serve the latest preconf state, before a
/// gossip protocol is implemented in op-node. Historical state will not be served from this API
#[rpc(client, server, namespace = "eth")]
pub trait EthApi {
    /// Sends signed transaction, returning its hash
    #[method(name = "sendRawTransaction")]
    async fn send_raw_transaction(&self, bytes: Bytes) -> RpcResult<B256>;

    // STORE

    /// Returns the receipt of a transaction by transaction hash
    #[method(name = "getTransactionReceipt")]
    async fn transaction_receipt(&self, hash: B256) -> RpcResult<Option<OpTransactionReceipt>>;

    /// Returns a block with a given identifier
    #[method(name = "getBlockByNumber")]
    async fn block_by_number(&self, number: BlockNumberOrTag, full: bool) -> RpcResult<Option<OpRpcBlock>>;

    /// Returns information about a block by hash.
    #[method(name = "getBlockByHash")]
    async fn block_by_hash(&self, hash: B256, full: bool) -> RpcResult<Option<OpRpcBlock>>;

    // DB

    /// Returns the number of most recent block
    #[method(name = "blockNumber")]
    async fn block_number(&self) -> RpcResult<U256>;

    /// Returns the nonce of a given address at a given block number.
    #[method(name = "getTransactionCount")]
    async fn transaction_count(&self, address: Address, block_number: Option<BlockId>) -> RpcResult<U256>;

    /// Returns the balance of the account of given address.
    #[method(name = "getBalance")]
    async fn balance(&self, address: Address, block_number: Option<BlockId>) -> RpcResult<U256>;
}

#[rpc(client, server, namespace = "eth")]
pub trait MinimalEthApi {
    /// Sends signed transaction, returning its hash
    #[method(name = "sendRawTransaction")]
    async fn send_raw_transaction(&self, bytes: Bytes) -> RpcResult<B256>;
}
