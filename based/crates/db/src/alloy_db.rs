use std::{future::IntoFuture, sync::Arc};

use alloy_eips::BlockId;
use alloy_primitives::{Address, B256, U256};
use alloy_provider::{
    network::{primitives::HeaderResponse, BlockResponse},
    Network, Provider,
};
use alloy_transport::{Transport, TransportError};
use revm::DatabaseRef;
use revm_primitives::{db::Database, AccountInfo, Bytecode};
use tokio::runtime::Runtime;

/// An alloy-powered REVM [Database].
///
/// When accessing the database, it'll use the given provider to fetch the corresponding account's data.
#[derive(Debug)]
pub struct AlloyDB<T: Transport + Clone, N: Network, P: Provider<T, N>> {
    /// The provider to fetch the data from.
    provider: P,
    /// The block number on which the queries will be based on.
    block_number: BlockId,
    /// handle to the tokio runtime
    rt: Arc<Runtime>,
    _marker: std::marker::PhantomData<fn() -> (T, N)>,
}

impl<T: Transport + Clone, N: Network, P: Provider<T, N>> AlloyDB<T, N, P> {
    /// Create a new AlloyDB instance, with a [Provider] and a block.
    ///
    /// Returns `None` if no tokio runtime is available or if the current runtime is a current-thread runtime.
    pub fn new(provider: P, block_number: BlockId, rt: Arc<Runtime>) -> Option<Self> {
        Some(Self { provider, block_number, rt, _marker: std::marker::PhantomData })
    }

    /// Set the block number on which the queries will be based on.
    pub fn set_block_number(&mut self, block_number: BlockId) {
        self.block_number = block_number;
    }
}

impl<T: Transport + Clone, N: Network, P: Provider<T, N>> DatabaseRef for AlloyDB<T, N, P> {
    type Error = TransportError;

    fn basic_ref(&self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        let (nonce, balance, code) = self.rt.block_on(async {
            let (nonce, balance, code) = tokio::join!(
                self.provider.get_transaction_count(address).block_id(self.block_number).into_future(),
                self.provider.get_balance(address).block_id(self.block_number).into_future(),
                self.provider.get_code_at(address).block_id(self.block_number).into_future()
            );
            Result::<_, TransportError>::Ok((nonce?, balance?, code?))
        })?;

        let code = Bytecode::new_raw(code.0.into());
        let code_hash = code.hash_slow();

        Ok(Some(AccountInfo::new(balance, nonce, code_hash, code)))
    }

    fn block_hash_ref(&self, number: u64) -> Result<B256, Self::Error> {
        let block = self.rt.block_on(
            self.provider
                // SAFETY: We know number <= u64::MAX, so we can safely convert it to u64
                .get_block_by_number(number.into(), false.into()),
        )?;
        // SAFETY: If the number is given, the block is supposed to be finalized, so unwrapping is safe.
        Ok(B256::new(*block.unwrap().header().hash()))
    }

    fn code_by_hash_ref(&self, _code_hash: B256) -> Result<Bytecode, Self::Error> {
        panic!("This should not be called, as the code is already loaded");
        // This is not needed, as the code is already loaded with basic_ref
    }

    fn storage_ref(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
        let f = self.provider.get_storage_at(address, index).block_id(self.block_number);
        let slot_val = self.rt.block_on(f.into_future())?;
        Ok(slot_val)
    }
}

impl<T: Transport + Clone, N: Network, P: Provider<T, N>> Database for AlloyDB<T, N, P> {
    type Error = TransportError;

    #[inline]
    fn basic(&mut self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        <Self as DatabaseRef>::basic_ref(self, address)
    }

    #[inline]
    fn code_by_hash(&mut self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        <Self as DatabaseRef>::code_by_hash_ref(self, code_hash)
    }

    #[inline]
    fn storage(&mut self, address: Address, index: U256) -> Result<U256, Self::Error> {
        <Self as DatabaseRef>::storage_ref(self, address, index)
    }

    #[inline]
    fn block_hash(&mut self, number: u64) -> Result<B256, Self::Error> {
        <Self as DatabaseRef>::block_hash_ref(self, number)
    }
}

#[cfg(test)]
mod tests {
    use alloy_provider::ProviderBuilder;

    use super::*;

    #[test]
    #[ignore = "flaky RPC"]
    fn can_get_basic() {
        let client = ProviderBuilder::new()
            .on_http("https://mainnet.infura.io/v3/c60b0bb42f8a4c6481ecd229eddaca27".parse().unwrap());
        let alloydb = AlloyDB::new(client, BlockId::from(16148323), Arc::new(Runtime::new().unwrap()));

        // ETH/USDT pair on Uniswap V2
        let address: Address = "0x0d4a11d5EEaaC28EC3F61d100daF4d40471f1852".parse().unwrap();

        let acc_info = alloydb.unwrap().basic_ref(address).unwrap().unwrap();
        assert!(acc_info.exists());
    }
}
