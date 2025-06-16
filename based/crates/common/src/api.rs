use std::collections::HashMap;

use alloy_eips::eip7685::RequestsOrHash;
use alloy_primitives::{Address, B256, Bytes, U256};
use alloy_rpc_types::{
    BlockId, BlockNumberOrTag,
    engine::{ExecutionPayloadV3, ForkchoiceState, ForkchoiceUpdated, PayloadId, PayloadStatus},
};
use jsonrpsee::proc_macros::rpc;
use op_alloy_consensus::OpTxEnvelope;
use op_alloy_rpc_types::OpTransactionReceipt;
use op_alloy_rpc_types_engine::{
    OpExecutionPayloadEnvelopeV3, OpExecutionPayloadEnvelopeV4, OpExecutionPayloadV4, OpPayloadAttributes,
};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::communication::messages::RpcResult;

pub const PORTAL_CAPABILITIES: &[&str] = &[
    "engine_forkchoiceUpdatedV3",
    "engine_getPayloadV3",
    "engine_getPayloadV4",
    "engine_newPayloadV3",
    "engine_newPayloadV4",
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

    #[method(name = "newPayloadV4")]
    async fn new_payload_v4(
        &self,
        payload: OpExecutionPayloadV4,
        versioned_hashes: Vec<B256>,
        parent_beacon_block_root: B256,
        _execution_requests: RequestsOrHash,
    ) -> RpcResult<PayloadStatus>;
    /// Used to fetch an execution payload from a previous `payload_id` set in `forkchoiceUpdatedV3`
    #[method(name = "getPayloadV3")]
    async fn get_payload_v3(&self, payload_id: PayloadId) -> RpcResult<OpExecutionPayloadEnvelopeV3> {
        let execution_payload = self.get_payload_v4(payload_id).await?;

        Ok(OpExecutionPayloadEnvelopeV3 {
            execution_payload: execution_payload.execution_payload.payload_inner,
            block_value: execution_payload.block_value,
            blobs_bundle: execution_payload.blobs_bundle,
            should_override_builder: execution_payload.should_override_builder,
            parent_beacon_block_root: execution_payload.parent_beacon_block_root,
        })
    }

    #[method(name = "getPayloadV4")]
    async fn get_payload_v4(&self, payload_id: PayloadId) -> RpcResult<OpExecutionPayloadEnvelopeV4>;
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

    /// Returns the current blocknumber and corresponding gateway url and address
    #[method(name = "registerGateway")]
    async fn register_gateway(&self, gateway: (Url, Address, B256)) -> RpcResult<()>;
}

#[rpc(client, server, namespace = "portal")]
pub trait PortalApi {
    /// The network id of the l2
    #[method(name = "l2ChainId")]
    async fn l2_chain_id(&self) -> RpcResult<u64>;
    /// The network id of the l1
    #[method(name = "l1ChainId")]
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

    // Heartbeat API
    #[method(name = "heartbeat")]
    async fn heartbeat(&self, jwt_secret: String) -> RpcResult<()>;
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

#[rpc(client, server, namespace = "opp2p")]
pub trait OpNodeP2PApi {
    /// The rollup config of the op-node
    #[method(name = "self")]
    async fn peer_info(&self) -> RpcResult<OpPeerInfo>;
    #[method(name = "peers")]
    async fn peers(&self, _t: bool) -> RpcResult<OpPeers>;
}

#[rpc(client, server, namespace = "admin")]
pub trait OpGethAdminApi {
    /// The rollup config of the op-node
    #[method(name = "nodeInfo")]
    async fn node_info(&self) -> RpcResult<OpGethInfo>;
    #[method(name = "peers")]
    async fn peers(&self) -> RpcResult<Vec<OpGethPeer>>;
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
    pub batch_inbox_address: Address,
    pub deposit_contract_address: Address,
    pub l1_system_config_address: Address,
    pub protocol_versions_address: Address,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Genesis {
    pub l1: BlockRef,
    pub l2: BlockRef,
    pub l2_time: u64,
    pub system_config: SystemConfig,
}

#[derive(Clone, Default, Serialize, Deserialize, Debug)]
pub struct BlockRef {
    pub hash: B256,
    pub number: u64,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SystemConfig {
    pub batcher_addr: Address,
    pub overhead: String,
    pub scalar: String,
    pub gas_limit: u64,
}

#[derive(Clone, Default, Serialize, Deserialize, Debug)]
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
    pub queued_unsafe_l2: Option<L2Block>,
    pub engine_sync_target: Option<L2Block>,
}

#[derive(Clone, Default, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct L1Block {
    pub hash: B256,
    pub number: u64,
    pub parent_hash: B256,
    pub timestamp: u64,
}

#[derive(Clone, Default, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct L2Block {
    pub hash: B256,
    pub number: u64,
    pub parent_hash: B256,
    pub timestamp: u64,
    #[serde(rename = "l1origin")]
    pub l1_origin: BlockRef,
    pub sequence_number: u64,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct OpPeers {
    pub total_connected: u64,
    pub peers: HashMap<String, OpPeerInfo>, // map keyed by peer-id strings

    #[serde(rename = "bannedPeers")]
    pub banned_peers: Vec<String>,
    #[serde(rename = "bannedIPS")]
    pub banned_ips: Vec<String>,
    #[serde(rename = "bannedSubnets")]
    pub banned_subnets: Vec<String>,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct OpPeerInfo {
    #[serde(alias = "peerID")]
    pub peer_id: String,
    #[serde(alias = "nodeID")]
    pub node_id: String,
    pub user_agent: String,
    pub protocol_version: String,
    #[serde(alias = "ENR")]
    pub enr: String,
    pub addresses: Vec<String>,
    pub protocols: Option<Value>,
    pub connectedness: u8,
    pub direction: u8,
    pub protected: bool,
    #[serde(alias = "chainID")]
    pub chain_id: u64,
    pub latency: f64,
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
    pub total: f64,
    pub blocks: BlockScores,
    #[serde(rename = "IPColocationFactor")]
    pub ip_colocation_factor: f64,
    pub behavioral_penalty: f64,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct BlockScores {
    pub time_in_mesh: f64,
    pub first_message_deliveries: f64,
    pub mesh_message_deliveries: f64,
    pub invalid_message_deliveries: f64,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ReqRespScore {
    pub valid_responses: f64,
    pub error_responses: f64,
    pub rejected_payloads: f64,
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
    pub terminal_total_difficulty: Option<u64>,
    pub terminal_total_difficulty_passed: Option<bool>,
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

#[derive(Clone, Default, Debug, Deserialize, Serialize)]
pub struct OpGethPeer {
    pub enr: Option<String>,
    pub enode: String,
    pub id: String,
    pub name: String,
    pub caps: Vec<String>,
    pub network: GethPeerNetwork,
    pub protocols: PeerProtocols,
}

#[derive(Clone, Default, Debug, Deserialize, Serialize)]
pub struct GethPeerNetwork {
    #[serde(alias = "localAddress")]
    pub local_address: String,
    #[serde(alias = "remoteAddress")]
    pub remote_address: String,
    pub inbound: bool,
    pub trusted: bool,
    #[serde(alias = "static")]
    pub static_str: bool, // `static` is a reserved keyword in Rust, so we need to use `static_`
}

#[derive(Clone, Default, Debug, Deserialize, Serialize)]
pub struct PeerProtocols {
    pub eth: EthVersion,
    pub snap: SnapVersion,
}

#[derive(Clone, Default, Debug, Deserialize, Serialize)]
pub struct EthVersion {
    pub version: u32,
}

#[derive(Clone, Default, Debug, Deserialize, Serialize)]
pub struct SnapVersion {
    pub version: Option<u32>,
}

#[cfg(test)]
pub mod test {
    use super::*;
    #[test]
    fn parse_op_node_peer_info() {
        assert_ne!(serde_json::from_str::<OpPeerInfo>("{\"ENR\":\"enr:-J-4QKMgVCRicuzgRSXF--kcfNcSb3el3gnK0VTKH5IqfAnjY096UPHcnpeOkYf8Y6hdbhFbjIoRcdMxKgy1QOftlZGGAZavZyU2gmlkgnY0gmlwhKwfGmOHb3BzdGFja4Xkq4MBAIlzZWNwMjU2azGhA38YA_8AH2SrzzVprDUjXbyv88AJION0F_UdgJmIk7v2g3RjcIIjK4N1ZHCCIys\",\"addresses\":[\"/ip4/127.0.0.1/tcp/9003/p2p/16Uiu2HAmMD7NHS98BXCoNuDGuS1zueMF5LQdiexCKyjJ97frMu6M\",\"/ip4/172.31.26.99/tcp/9003/p2p/16Uiu2HAmMD7NHS98BXCoNuDGuS1zueMF5LQdiexCKyjJ97frMu6M\"],\"chainID\":0,\"connectedness\":0,\"direction\":0,\"gossipBlocks\":true,\"latency\":0,\"nodeID\":\"ca1451eb9482746566a92fbde4bcb7c646d23bfceb21f66d4d79e1e0f0819cfc\",\"peerID\":\"16Uiu2HAmMD7NHS98BXCoNuDGuS1zueMF5LQdiexCKyjJ97frMu6M\",\"protected\":false,\"protocolVersion\":\"\",\"protocols\":null,\"scores\":{\"gossip\":{\"IPColocationFactor\":0,\"behavioralPenalty\":0,\"blocks\":{\"firstMessageDeliveries\":0,\"invalidMessageDeliveries\":0,\"meshMessageDeliveries\":0,\"timeInMesh\":0},\"total\":0},\"reqResp\":{\"errorResponses\":0,\"rejectedPayloads\":0,\"validResponses\":0}},\"userAgent\":\"\"}").unwrap().peer_id, "");
    }

    #[test]
    fn parse_geth_info() {
        assert_ne!(serde_json::from_str::<OpGethInfo>("{\"enode\":\"enode://36c3170ea04471fb52a8c0d4f8f06da660e2e7388959089844269d5b790be4215ebc5052892cf40be08ffc19bcd7439db6fdf713c6fb6d0f7960906ce088de52@57.133.217.139:30303?discport=1089\",\"enr\":\"enr:-Ku4QA9W-NTtseU4M0OBXgdVBkpSEKP_D3l-TicPAyBACAdLPVjNRD2oMpBeK8_z-Y7Xl31iovo-O0eoJ4HII6d0CS6GAZbFKQKgg2V0aMfGhBXQ1LeAgmlkgnY0gmlwhDmF2YuJc2VjcDI1NmsxoQI2wxcOoERx-1KowNT48G2mYOLnOIlZCJhEJp1beQvkIYRzbmFwwIN0Y3CCdl-DdWRwggRBhHVkcDaCdl8\",\"id\":\"4b259315183c61074e998c46eec2689f2cc321e81226b4bea0c83ee57a00c96f\",\"ip\":\"57.133.217.139\",\"listenAddr\":\"[::]:30303\",\"name\":\"Geth/v1.101411.8-rc.1-374d61f9-20250211/linux-amd64/go1.23.6\",\"ports\":{\"discovery\":1089,\"listener\":30303},\"protocols\":{\"eth\":{\"config\":{\"arrowGlacierBlock\":0,\"bedrockBlock\":0,\"berlinBlock\":0,\"byzantiumBlock\":0,\"cancunTime\":0,\"canyonTime\":0,\"chainId\":2151908,\"constantinopleBlock\":0,\"depositContractAddress\":\"0x0000000000000000000000000000000000000000\",\"ecotoneTime\":0,\"eip150Block\":0,\"eip155Block\":0,\"eip158Block\":0,\"fjordTime\":0,\"graniteTime\":0,\"grayGlacierBlock\":0,\"holoceneTime\":0,\"homesteadBlock\":0,\"istanbulBlock\":0,\"londonBlock\":0,\"mergeNetsplitBlock\":0,\"muirGlacierBlock\":0,\"optimism\":{\"eip1559Denominator\":50,\"eip1559DenominatorCanyon\":250,\"eip1559Elasticity\":6},\"petersburgBlock\":0,\"regolithTime\":0,\"shanghaiTime\":0,\"terminalTotalDifficulty\":0},\"difficulty\":0,\"genesis\":\"0xf81cfade9797c41a311da5bb09fbc77fa481bfdaff40e9fcc99a4dc43453b1b3\",\"head\":\"0x4469e2e3a10785af7b05b69cbbeae7bd3c3c47b3df81dfb98dfee974918350b5\",\"network\":2151908},\"snap\":{}}}").unwrap().id, "");
    }

    #[test]
    fn parse_geth_peers() {
        assert!(serde_json::from_str::<OpGethPeer>("{\"enr\":\"enr:-KO4QPfDpnj33Qcejr3m7rk8mMA7nXzBrOvm5bpsMfuun09Nfgdv1O8qqGhl9v_e69b2wAogFjUKK8ZXReI8pgPS9ZiGAZbbGWtgg2V0aMfGhFmJ9hSAgmlkgnY0gmlwhBK5xzOJc2VjcDI1NmsxoQI-TZICg15Lqr3j5KRiqSVTVwuV9FQJpvB9GrAMB7w5pYRzbmFwwIN0Y3CCdl-DdWRwgnZf\",\"enode\":\"enode://3e4d9202835e4baabde3e4a462a92553570b95f45409a6f07d1ab00c07bc39a52dd710f098008ae5974aa625edeb46734e3634f0c9d3ab24bb35e37d12f1ac18@18.185.199.51:30303\",\"id\":\"1b83fb2f39f76c9926dd1690b3570e956f3a43e599acb89d28c739022db5d887\",\"name\":\"Geth/v1.101411.8-rc.1-374d61f9-20250211/linux-amd64/go1.23.6\",\"caps\":[\"eth/68\",\"snap/1\"],\"network\":{\"localAddress\":\"192.168.1.33:35502\",\"remoteAddress\":\"18.185.199.51:30303\",\"inbound\":false,\"trusted\":false,\"static\":false},\"protocols\":{\"eth\":{\"version\":68},\"snap\":{\"version\":1}}}").inspect_err(|e| panic!("{e}")).is_ok());
    }
}
