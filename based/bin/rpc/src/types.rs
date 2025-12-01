use alloy_eips::BlockId;
use alloy_primitives::{Address, B256, Bytes, U256};
use alloy_provider::RootProvider;
use alloy_rpc_types::{BlockOverrides, state::StateOverride};
use jsonrpsee::{core::RpcResult, proc_macros::rpc};
use op_alloy_network::Optimism;
use op_alloy_rpc_types::{OpTransactionReceipt, OpTransactionRequest};

pub type OpRootProvider = RootProvider<Optimism>;

// taken from https://github.com/gattaca-com/based-op/blob/397d48b73d088f40721ae0ba002d251dcf6f38cc/based/crates/common/src/api.rs#L91-L128
#[rpc(client, server, namespace = "eth")]
pub trait EthApi {
    /// Sends signed transaction, returning its hash
    #[method(name = "sendRawTransaction")]
    async fn send_raw_transaction(&self, bytes: Bytes) -> RpcResult<B256>;

    /// Sends signed transaction, waiting for it to be mined and returning the receipt
    #[method(name = "sendRawTransactionSync")]
    async fn send_raw_transaction_sync(&self, bytes: Bytes, timeout_ms: u64) -> RpcResult<OpTransactionReceipt>;

    /// Returns the receipt of a transaction by transaction hash
    #[method(name = "getTransactionReceipt")]
    async fn transaction_receipt(&self, hash: B256) -> RpcResult<Option<OpTransactionReceipt>>;

    // /// Returns a block with a given identifier
    // #[method(name = "getBlockByNumber")]
    // async fn block_by_number(&self, number: BlockNumberOrTag, full: bool) -> RpcResult<Option<OpRpcBlock>>;

    // /// Returns information about a block by hash.
    // #[method(name = "getBlockByHash")]
    // async fn block_by_hash(&self, hash: B256, full: bool) -> RpcResult<Option<OpRpcBlock>>;

    /// Returns the number of most recent block
    #[method(name = "blockNumber")]
    async fn block_number(&self) -> RpcResult<U256>;

    /// Returns the nonce of a given address at a given block number.
    #[method(name = "getTransactionCount")]
    async fn transaction_count(&self, address: Address, block_number: Option<BlockId>) -> RpcResult<U256>;

    /// Returns the balance of the account of given address.
    #[method(name = "getBalance")]
    async fn balance(&self, address: Address, block_number: Option<BlockId>) -> RpcResult<U256>;

    #[method(name = "call")]
    async fn call(
        &self,
        transaction: OpTransactionRequest,
        block_number: Option<BlockId>,
        state_overrides: Option<StateOverride>,
        block_overrides: Option<BlockOverrides>,
    ) -> RpcResult<Bytes>;
}
