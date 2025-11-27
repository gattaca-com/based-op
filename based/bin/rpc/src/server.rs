use std::{sync::Arc, time::Duration};

use alloy_eips::{BlockId, BlockNumberOrTag};
use alloy_primitives::{
    Address, B256, Bytes, U256,
    map::foldhash::{HashMap, HashMapExt},
};
use alloy_provider::Provider;
use alloy_rpc_types::{
    BlockOverrides, Header,
    state::{AccountOverride, StateOverride},
};
use bop_common::p2p::{DetailedStateChange, EnvV0, FragV0, SealV0, StateUpdate};
use eyre::Result;
use jsonrpsee::{
    core::{RpcResult, async_trait},
    http_client::HttpClientBuilder,
    types::ErrorObject,
};
use op_alloy_rpc_types::{OpTransactionReceipt, OpTransactionRequest};
use parking_lot::RwLock;
use reqwest::Url;
use tracing::error;

use crate::{
    middleware::RpcClient,
    types::{EthApiServer, OpRootProvider},
    unsealed_block::{UnsealedBlock, UnsealedBlockStack},
};

#[derive(Clone)]
pub struct Server {
    unsealed_stack: Arc<RwLock<UnsealedBlockStack>>,
    provider: OpRootProvider,
    tx_receiver_provider: OpRootProvider,
}

impl Server {
    pub fn new(provider: OpRootProvider, tx_receiver_provider: OpRootProvider) -> Self {
        Self { unsealed_stack: Arc::new(RwLock::new(UnsealedBlockStack::new())), provider, tx_receiver_provider }
    }

    pub fn on_env(&self, env: EnvV0) {
        let mut unsealed_block = self.unsealed_stack.upgradable_read();
        if unsealed_block.blocks.is_empty() || unsealed_block.blocks.back().unwrap().env.number + 1 == env.number {
            unsealed_block.with_upgraded(|blocks| {
                blocks.blocks.push_back(UnsealedBlock {
                    env,
                    current_frag: None,
                    transaction_count_diff: HashMap::new(),
                    receipts: HashMap::new(),
                    balances: HashMap::new(),
                    state_changes: HashMap::new(),
                    seal: None,
                });
            });
        } else {
            error!("expected block number");
        }
    }

    pub fn on_header(&self, header: Header) {
        let mut unsealed_block = self.unsealed_stack.upgradable_read();
        while !unsealed_block.blocks.is_empty() && unsealed_block.blocks.front().unwrap().env.number <= header.number {
            unsealed_block.with_upgraded(|blocks| {
                blocks.blocks.pop_front();
                blocks.root_provider_block_number = Some(header.number);
            });
        }
    }

    pub fn on_seal(&self, seal: SealV0) {
        let mut unsealed_block = self.unsealed_stack.upgradable_read();
        if unsealed_block.blocks.is_empty() {
            // error!("trying to seal a block but there is no unsealed block");
            return;
        }
        let last_block = unsealed_block.blocks.back().unwrap(); // unwrap is safe because we just checked that the stack is not empty
        if last_block.env.number != seal.block_number {
            error!(
                "trying to seal block number {} but the last unsealed block is number {}",
                seal.block_number, last_block.env.number
            );
            return;
        }
        unsealed_block.with_upgraded(|blocks| {
            blocks.blocks.back_mut().unwrap().apply_seal(seal);
        });
    }

    pub fn on_frag(&self, frag: FragV0, state_update: Option<StateUpdate>) {
        let mut unsealed_block = self.unsealed_stack.upgradable_read();
        if unsealed_block.blocks.is_empty() {
            return;
        }
        let last_block = unsealed_block.blocks.back().unwrap(); // unwrap is safe because we just checked that the stack is not empty
        if last_block.env.number != frag.block_number {
            error!(
                "trying to apply frag for block number {} but the last unsealed block is number {}",
                frag.block_number, last_block.env.number
            );
            return;
        }
        unsealed_block.with_upgraded(|blocks| {
            blocks.blocks.back_mut().unwrap().apply_frag(frag, state_update);
        });
    }

    pub async fn get_transaction_count(&self, address: Address) -> Result<u64> {
        let (transaction_count_diff, root_provider_block_number) = {
            let stack = self.unsealed_stack.read();
            (stack.get_transaction_count_diff(address), stack.root_provider_block_number)
        };

        if let Some(root_provider_block_number) = root_provider_block_number {
            let transaction_count = self
                .provider
                .get_transaction_count(address)
                .block_id(BlockId::number(root_provider_block_number))
                .await?;
            return Ok(transaction_count_diff + transaction_count);
        }
        let transaction_count = self.provider.get_transaction_count(address).await?;
        Ok(transaction_count_diff + transaction_count)
    }

    pub async fn get_balance(&self, address: Address) -> Result<U256> {
        let balance = self.unsealed_stack.read().get_balance(address);
        if let Some(balance) = balance {
            return Ok(balance);
        }
        let balance = self.provider.get_balance(address).await?;
        Ok(balance)
    }

    pub async fn get_receipt(&self, hash: B256) -> Result<Option<OpTransactionReceipt>> {
        if let Some(receipt) = self.unsealed_stack.read().get_receipt(hash) {
            return Ok(Some(receipt));
        }
        let receipt = self.tx_receiver_provider.get_transaction_receipt(hash).await?;
        Ok(receipt)
    }

    pub async fn block_number(&self) -> Result<u64> {
        if let Some(block_number) = self.unsealed_stack.read().block_number() {
            Ok(block_number)
        } else {
            Ok(self.provider.get_block_number().await?)
        }
    }

    pub fn base_block_number(&self) -> Option<u64> {
        self.unsealed_stack.read().root_provider_block_number
    }

    pub fn get_state_changes(&self) -> HashMap<Address, DetailedStateChange> {
        self.unsealed_stack.read().get_state_changes()
    }
}

#[async_trait]
impl EthApiServer for Server {
    async fn send_raw_transaction(&self, bytes: Bytes) -> RpcResult<B256> {
        let hash = self.tx_receiver_provider.send_raw_transaction(&bytes).await.map_err(|e| {
            ErrorObject::owned(
                jsonrpsee::types::error::INTERNAL_ERROR_CODE,
                "Failed to send transaction",
                Some(e.to_string()),
            )
        })?;
        Ok(*hash.tx_hash())
    }

    async fn transaction_receipt(&self, hash: B256) -> RpcResult<Option<OpTransactionReceipt>> {
        let receipt = self.get_receipt(hash).await.map_err(|e| {
            ErrorObject::owned(
                jsonrpsee::types::error::INTERNAL_ERROR_CODE,
                "Failed to get transaction receipt",
                Some(e.to_string()),
            )
        })?;
        Ok(receipt)
    }

    // async fn block_by_number(&self, number: BlockNumberOrTag, full: bool) -> RpcResult<Option<OpRpcBlock>> {
    //     todo!()
    // }

    // async fn block_by_hash(&self, hash: B256, full: bool) -> RpcResult<Option<OpRpcBlock>> {
    //     todo!()
    // }

    async fn block_number(&self) -> RpcResult<U256> {
        let block_number = self.block_number().await.map_err(|e| {
            ErrorObject::owned(
                jsonrpsee::types::error::INTERNAL_ERROR_CODE,
                "Failed to get block number",
                Some(e.to_string()),
            )
        })?;
        Ok(U256::from(block_number))
    }

    async fn transaction_count(&self, address: Address, block_number: Option<BlockId>) -> RpcResult<U256> {
        match block_number {
            Some(BlockId::Number(BlockNumberOrTag::Pending)) => {
                let block_number = self.block_number().await.map_err(|e| {
                    ErrorObject::owned(
                        jsonrpsee::types::error::INTERNAL_ERROR_CODE,
                        "Failed to get block number",
                        Some(e.to_string()),
                    )
                })?;
                Ok(U256::from(block_number))
            }
            _ => {
                let transaction_count = self
                    .provider
                    .get_transaction_count(address)
                    .block_id(block_number.unwrap_or_default())
                    .await
                    .map_err(|e| {
                        ErrorObject::owned(
                            jsonrpsee::types::error::INTERNAL_ERROR_CODE,
                            "Failed to get transaction count",
                            Some(e.to_string()),
                        )
                    })?;
                Ok(U256::from(transaction_count))
            }
        }
    }

    async fn balance(&self, address: Address, block_number: Option<BlockId>) -> RpcResult<U256> {
        if let Some(BlockId::Number(BlockNumberOrTag::Pending)) = block_number {
            if let Ok(balance) = self.get_balance(address).await {
                return Ok(U256::from(balance));
            }
        }

        let balance =
            self.provider.get_balance(address).block_id(block_number.unwrap_or_default()).await.map_err(|e| {
                ErrorObject::owned(
                    jsonrpsee::types::error::INTERNAL_ERROR_CODE,
                    "Failed to get balance",
                    Some(e.to_string()),
                )
            })?;
        Ok(U256::from(balance))
    }

    async fn call(
        &self,
        transaction: OpTransactionRequest,
        block_number: Option<BlockId>,
        state_overrides: Option<StateOverride>,
        block_overrides: Option<BlockOverrides>,
    ) -> RpcResult<Bytes> {
        if let Some(BlockId::Number(BlockNumberOrTag::Pending)) = block_number {
            if block_overrides.is_some() {
                return Err(ErrorObject::owned(
                    jsonrpsee::types::error::INTERNAL_ERROR_CODE,
                    "Block overrides are not supported",
                    Some("Block overrides are not supported".to_string()),
                ));
            }

            if state_overrides.is_some() {
                return Err(ErrorObject::owned(
                    jsonrpsee::types::error::INTERNAL_ERROR_CODE,
                    "State overrides are not supported",
                    Some("State overrides are not supported".to_string()),
                ));
            }

            let mut state_overrides_full = StateOverride::default();

            let base_block_number = self.base_block_number().unwrap_or_default();
            let state_overrides_unsealed_block = self.get_state_changes();
            for (address, state_change) in state_overrides_unsealed_block.iter() {
                let account = state_overrides_full.entry(*address).or_insert_with(AccountOverride::default);
                account.balance = Some(state_change.balance);
                account.nonce = Some(state_change.nonce);
                if !state_change.storage.is_empty() {
                    if account.state_diff.is_none() {
                        account.state_diff = Some(Default::default());
                    }
                    let account_storage = account.state_diff.as_mut().unwrap();
                    for (slot, value) in state_change.storage.iter() {
                        account_storage.insert((*slot).into(), (*value).into());
                    }
                }
            }

            let result = self
                .provider
                .call(transaction)
                .block(BlockId::number(base_block_number))
                .overrides(state_overrides_full)
                .await;
            match result {
                Ok(result) => Ok(result),
                Err(e) => Err(ErrorObject::owned(
                    jsonrpsee::types::error::INTERNAL_ERROR_CODE,
                    "Failed to call",
                    Some(e.to_string()),
                )),
            }
        } else {
            let request = self
                .provider
                .call(transaction)
                .block(block_number.unwrap_or_default())
                .overrides_opt(state_overrides)
                .with_block_overrides_opt(block_overrides);
            let result = request.await;
            match result {
                Ok(result) => Ok(result),
                Err(e) => Err(ErrorObject::owned(
                    jsonrpsee::types::error::INTERNAL_ERROR_CODE,
                    "Failed to call",
                    Some(e.to_string()),
                )),
            }
        }
    }
}

pub fn create_client(url: Url, timeout: Duration) -> eyre::Result<RpcClient> {
    let client = HttpClientBuilder::default()
        .max_request_size(u32::MAX)
        .max_response_size(u32::MAX)
        .request_timeout(timeout)
        .build(url)?;
    Ok(client)
}
