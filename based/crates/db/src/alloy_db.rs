use std::{fmt::Debug, future::IntoFuture, sync::Arc};

use alloy_consensus::BlockHeader;
use alloy_eips::BlockId;
use alloy_primitives::{Address, B256, U256};
use alloy_provider::{
    network::{primitives::HeaderResponse, BlockResponse},
    Provider, RootProvider,
};
use alloy_transport::TransportError;
use alloy_transport_http::Http;
use op_alloy_network::Optimism;
use reth_db::DatabaseError;
use reth_optimism_primitives::{OpBlock, OpReceipt};
use reth_primitives::BlockWithSenders;
use reth_provider::{BlockExecutionOutput, ProviderError};
use reth_trie_common::updates::TrieUpdates;
use revm::{db::BundleState, DatabaseCommit, DatabaseRef};
use revm_primitives::{db::Database, Account, AccountInfo, Bytecode, HashMap};
use tokio::runtime::Runtime;

use crate::{DatabaseRead, DatabaseWrite, Error};

type AlloyProvider = RootProvider<Http<reqwest::Client>, Optimism>;

/// A stripped down version of [`revm::db::AlloyDB`].
///
/// When accessing the database, it'll use the given provider to fetch the corresponding account's data.
#[derive(Debug, Clone)]
pub struct AlloyDB {
    /// The provider to fetch the data from.
    provider: AlloyProvider,
    /// The block number on which the queries will be based on.
    block_number: BlockId,
    /// handle to the tokio runtime
    rt: Arc<Runtime>,
}

impl AlloyDB {
    /// Create a new AlloyDB instance, with a [Provider] and a block.
    /// We subtract 1 from the block number, as the state we want to fetch is the end of the previous block.
    pub fn new(provider: AlloyProvider, block_number: u64, rt: Arc<Runtime>) -> Self {
        let block_number = BlockId::from(block_number.saturating_sub(1));
        Self { provider, block_number, rt }
    }

    /// Set the block number on which the queries will be based on.
    ///
    /// We subtract 1 from the block number, as the state we want to fetch is the end of the previous block.
    pub fn set_block_number(&mut self, block_number: u64) {
        self.block_number = BlockId::from(block_number.saturating_sub(1));
    }
}

impl DatabaseRef for AlloyDB {
    type Error = ProviderError;

    fn basic_ref(&self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        let (nonce, balance, code) = self
            .rt
            .block_on(async {
                let (nonce, balance, code) = tokio::join!(
                    self.provider.get_transaction_count(address).block_id(self.block_number).into_future(),
                    self.provider.get_balance(address).block_id(self.block_number).into_future(),
                    self.provider.get_code_at(address).block_id(self.block_number).into_future()
                );
                Result::<_, TransportError>::Ok((nonce?, balance?, code?))
            })
            .map_err(|e| ProviderError::Database(DatabaseError::Other(e.to_string())))?;

        let code = Bytecode::new_raw(code.0.into());
        let code_hash = code.hash_slow();

        Ok(Some(AccountInfo::new(balance, nonce, code_hash, code)))
    }

    fn block_hash_ref(&self, number: u64) -> Result<B256, Self::Error> {
        let block = self
            .rt
            .block_on(
                self.provider
                    // SAFETY: We know number <= u64::MAX, so we can safely convert it to u64
                    .get_block_by_number(number.into(), false.into()),
            )
            .map_err(|e| ProviderError::Database(DatabaseError::Other(e.to_string())))?;
        // SAFETY: If the number is given, the block is supposed to be finalized, so unwrapping is safe.
        Ok(B256::new(*block.unwrap().header().hash()))
    }

    fn code_by_hash_ref(&self, _code_hash: B256) -> Result<Bytecode, Self::Error> {
        panic!("This should not be called, as the code is already loaded");
        // This is not needed, as the code is already loaded with basic_ref
    }

    fn storage_ref(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
        let f = self.provider.get_storage_at(address, index).block_id(self.block_number);
        let slot_val = self
            .rt
            .block_on(f.into_future())
            .map_err(|e| ProviderError::Database(DatabaseError::Other(e.to_string())))?;
        Ok(slot_val)
    }
}

impl Database for AlloyDB {
    type Error = ProviderError;

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

impl DatabaseCommit for AlloyDB {
    fn commit(&mut self, _: HashMap<Address, Account>) {
        // No-op, as we don't need to commit to the database.
    }
}

impl DatabaseRead for AlloyDB {
    /// Fetches the state
    fn calculate_state_root(&self, _: &BundleState) -> Result<(B256, TrieUpdates), Error> {
        debug_assert!(matches!(self.block_number, BlockId::Number(_)), "block_number should always be a number");

        let next_block = self.block_number.as_u64().expect("block number is valid") + 1;

        let root = self
            .rt
            .block_on(self.provider.get_block_by_number(next_block.into(), false.into()))
            .map_err(|e| Error::Other(e.to_string()))?
            .ok_or_else(|| Error::Other(format!("Block not found: {next_block}")))?
            .header()
            .state_root();

        Ok((root, TrieUpdates::default()))
    }

    /// Returns the current block head number.
    fn head_block_number(&self) -> Result<u64, Error> {
        debug_assert!(matches!(self.block_number, BlockId::Number(_)), "block_number should always be a number");
        Ok(self.block_number.as_u64().unwrap())
    }

    fn head_block_hash(&self) -> Result<B256, Error> {
        debug_assert!(matches!(self.block_number, BlockId::Number(_)), "block_number should always be a number");
        Ok(self.block_hash_ref(self.block_number.as_u64().unwrap())?)
    }
}

impl DatabaseWrite for AlloyDB {
    fn commit_block(
        &self,
        _block: &BlockWithSenders<OpBlock>,
        _block_execution_output: BlockExecutionOutput<OpReceipt>,
    ) -> Result<(), Error> {
        // No-op
        Ok(())
    }

    fn commit_block_unchecked(
        &self,
        _block: &BlockWithSenders<OpBlock>,
        _block_execution_output: BlockExecutionOutput<OpReceipt>,
        _trie_updates: TrieUpdates,
    ) -> Result<(), Error> {
        // No-op
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloy_provider::ProviderBuilder;
    use revm_primitives::address;

    use super::*;

    #[test]
    #[ignore = "flaky RPC"]
    fn can_get_basic() {
        let provider = ProviderBuilder::new()
            .network()
            .on_http("https://mainnet.infura.io/v3/c60b0bb42f8a4c6481ecd229eddaca27".parse().unwrap());
        let alloydb = AlloyDB::new(provider, 16148323, Arc::new(Runtime::new().unwrap()));

        // ETH/USDT pair on Uniswap V2
        let address = address!("0d4a11d5EEaaC28EC3F61d100daF4d40471f1852");

        let acc_info = alloydb.basic_ref(address).unwrap().unwrap();
        assert!(acc_info.exists());
    }
}
