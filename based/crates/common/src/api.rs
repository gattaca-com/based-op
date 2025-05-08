use alloy_primitives::{Address, B256, Bytes, U256};
use alloy_rpc_types::{
    BlockId, BlockNumberOrTag,
    engine::{ExecutionPayloadV3, ForkchoiceState, ForkchoiceUpdated, PayloadId, PayloadStatus},
};
use jsonrpsee::proc_macros::rpc;
use op_alloy_consensus::OpTxEnvelope;
use op_alloy_rpc_types::OpTransactionReceipt;
use op_alloy_rpc_types_engine::{OpExecutionPayloadEnvelopeV3, OpPayloadAttributes};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::communication::messages::RpcResult;

pub const PORTAL_CAPABILITIES: &[&str] = &[
    "engine_forkchoiceUpdatedV3",
    "engine_getPayloadV3",
    "engine_newPayloadV3",
    "eth_sendRawTransaction",
    // "eth_getTransactionReceipt",
    // "eth_getBlockByNumber",
    // "eth_getBlockByHash",
    // "eth_blockNumber",
    // "eth_getTransactionCount",
    // "eth_getBalance",
];

pub type OpRpcBlock = alloy_rpc_types::Block<OpTxEnvelope>;

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

#[rpc(client, server, namespace = "registry")]
pub trait RegistryApi {
    /// Returns the future blocknumber and corresponding gateway url and address
    #[method(name = "futureGateway")]
    async fn get_future_gateway(&self, n_blocks_into_future: u64) -> RpcResult<(u64, Url, Address, B256)>;

    /// Returns the current blocknumber and corresponding gateway url and address
    #[method(name = "currentGateway")]
    async fn current_gateway(&self) -> RpcResult<(u64, Url, Address, B256)> {
        self.get_future_gateway(0).await
    }

    /// Returns the current blocknumber and corresponding gateway url and address
    #[method(name = "registeredGateways")]
    async fn registered_gateways(&self) -> RpcResult<Vec<(Url, Address, B256)>>;
}

#[rpc(client, server, namespace = "portal")]
pub trait PortalApi {
    /// The network id of the l2
    #[method(name = "l2NetworkId")]
    async fn l2_chain_id(&self) -> RpcResult<u64>;
    /// The network id of the l1
    #[method(name = "l1NetworkId")]
    async fn l1_chain_id(&self) -> RpcResult<u64>;

    /// rollup.json file
    #[method(name = "fileRollup")]
    async fn file_rollup(&self) -> RpcResult<String>;
    /// genesis.json file
    #[method(name = "fileGenesis")]
    async fn file_genesis(&self) -> RpcResult<String>;

    /// The gossip static address string used by the op-node
    #[method(name = "opNodeGossipStatic")]
    async fn op_node_gossip_static(&self) -> RpcResult<String>;

    /// The enr that can be used to sync with the op-node
    #[method(name = "opNodeBootnodeEnr")]
    async fn op_node_bootnode_enr(&self) -> RpcResult<String>;

    /// The enode that can be used to sync with the op-geth
    #[method(name = "opGethBootnodeEnode")]
    async fn op_geth_bootnode_enode(&self) -> RpcResult<String>;
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct RollupConfig {
    pub genesis: Genesis,
    pub block_time: u64,
    pub max_sequencer_drift: u64,
    pub seq_window_size: u64,
    pub channel_timeout: u64,
    pub l1_chain_id: u64,
    pub l2_chain_id: u64,
    pub regolith_time: u64,
    pub canyon_time: u64,
    pub batch_inbox_address: String,
    pub deposit_contract_address: String,
    pub l1_system_config_address: String,
    pub protocol_versions_address: String,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Genesis {
    pub l1: BlockRef,
    pub l2: BlockRef,
    pub l2_time: u64,
    pub system_config: SystemConfig,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct BlockRef {
    pub hash: String,
    pub number: u64,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SystemConfig {
    pub batcher_addr: String,
    pub overhead: String,
    pub scalar: String,
    pub gas_limit: u64,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct SyncStatus {
    pub current_l1: L1Block,
    pub current_l1_finalized: L1Block,
    pub head_l1: L1Block,
    pub safe_l1: L1Block,
    pub finalized_l1: L1Block,
    pub unsafe_l2: L2Block,
    pub safe_l2: L2Block,
    pub finalized_l2: L2Block,
    pub pending_safe_l2: L2Block,
    pub queued_unsafe_l2: L2Block,
    pub engine_sync_target: L2Block,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct L1Block {
    pub hash: String,
    pub number: u64,
    pub parent_hash: String,
    pub timestamp: u64,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct L2Block {
    pub hash: String,
    pub number: u64,
    pub parent_hash: String,
    pub timestamp: u64,
    #[serde(rename = "l1origin")]
    pub l1_origin: BlockRef,
    pub sequence_number: u64,
}

#[rpc(client, server, namespace = "optimism")]
pub trait OpNodeApi {
    /// The rollup config of the op-node
    #[method(name = "rollupConfig")]
    async fn rollup_config(&self) -> RpcResult<RollupConfig>;

    /// The syncstatus of the op-node
    #[method(name = "syncStatus")]
    async fn sync_status(&self) -> RpcResult<SyncStatus>;
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PeerInfo {
    pub peer_id: String,
    pub node_id: String,
    pub user_agent: String,
    pub protocol_version: String,
    #[serde(rename = "ENR")]
    pub enr: String,
    pub addresses: Vec<String>,
    pub protocols: Option<Value>,
    pub connectedness: u8,
    pub direction: u8,
    pub protected: bool,
    pub chain_id: u64,
    pub latency: u64,
    pub gossip_blocks: bool,
    pub scores: Scores,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Scores {
    pub gossip: GossipScore,
    pub req_resp: ReqRespScore,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct GossipScore {
    pub total: u64,
    pub blocks: BlockScores,
    #[serde(rename = "IPColocationFactor")]
    pub ip_colocation_factor: u64,
    pub behavioral_penalty: u64,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct BlockScores {
    pub time_in_mesh: u64,
    pub first_message_deliveries: u64,
    pub mesh_message_deliveries: u64,
    pub invalid_message_deliveries: u64,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ReqRespScore {
    pub valid_responses: u64,
    pub error_responses: u64,
    pub rejected_payloads: u64,
}

#[rpc(client, server, namespace = "opp2p")]
pub trait OpNodeP2PApi {
    /// The rollup config of the op-node
    #[method(name = "self")]
    async fn peer_info(&self) -> RpcResult<PeerInfo>;
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct OpGethInfo {
    pub id: String,
    pub name: String,
    pub enode: String,
    pub enr: String,
    pub ip: String,
    pub ports: Ports,
    #[serde(rename = "listenAddr")]
    pub listen_addr: String,
    pub protocols: Protocols,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Ports {
    pub discovery: u16,
    pub listener: u16,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Protocols {
    pub eth: Eth,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Eth {
    pub network: u64,
    pub difficulty: u64,
    pub genesis: String,
    pub config: EthConfig,
    pub head: String,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct EthConfig {
    pub chain_id: u64,
    pub homestead_block: u64,
    pub eip150_block: u64,
    pub eip155_block: u64,
    pub eip158_block: u64,
    pub byzantium_block: u64,
    pub constantinople_block: u64,
    pub petersburg_block: u64,
    pub istanbul_block: u64,
    pub muir_glacier_block: u64,
    pub berlin_block: u64,
    pub london_block: u64,
    pub arrow_glacier_block: u64,
    pub gray_glacier_block: u64,
    pub merge_netsplit_block: u64,
    pub shanghai_time: u64,
    pub cancun_time: u64,
    pub bedrock_block: u64,
    pub regolith_time: u64,
    pub canyon_time: u64,
    pub ecotone_time: u64,
    pub fjord_time: u64,
    pub granite_time: u64,
    pub holocene_time: u64,
    pub terminal_total_difficulty: u64,
    pub terminal_total_difficulty_passed: bool,
    pub deposit_contract_address: String,
    pub optimism: OptimismConfig,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct OptimismConfig {
    pub eip1559_elasticity: u64,
    pub eip1559_denominator: u64,
    pub eip1559_denominator_canyon: u64,
}

#[rpc(client, server, namespace = "admin")]
pub trait OpGethAdminApi {
    /// The rollup config of the op-node
    #[method(name = "nodeInfo")]
    async fn node_info(&self) -> RpcResult<OpGethInfo>;
}
