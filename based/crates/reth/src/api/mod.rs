use op_alloy_network::Optimism;
use op_alloy_rpc_types::OpTransactionReceipt;
use reth_rpc_eth_api::RpcReceipt;

pub mod engine;
pub mod eth;

pub(crate) trait ToRpc {
    type RpcVariant;

    fn as_rpc(&self) -> Self::RpcVariant;

    fn into_rpc(self) -> Self::RpcVariant;
}

impl ToRpc for OpTransactionReceipt {
    type RpcVariant = RpcReceipt<Optimism>;

    fn as_rpc(&self) -> Self::RpcVariant {
        RpcReceipt::<Optimism>::from(self.clone())
    }

    fn into_rpc(self) -> Self::RpcVariant {
        RpcReceipt::<Optimism>::from(self)
    }
}
