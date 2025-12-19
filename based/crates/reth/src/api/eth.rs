use std::{collections::HashSet, sync::Arc, time::Duration};

use alloy_eips::{BlockId, BlockNumberOrTag};
use alloy_primitives::{Address, TxHash, U256};
use alloy_rpc_types::{
    BlockOverrides, Filter, FilterBlockOption, Log,
    simulate::{SimBlock, SimulatePayload, SimulatedBlock},
    state::{EvmOverrides, StateOverride, StateOverridesBuilder},
};
use arc_swap::ArcSwapOption;
use jsonrpsee::{
    core::{RpcResult, async_trait},
    proc_macros::rpc,
    types::{ErrorObject, ErrorObjectOwned, error::INVALID_PARAMS_CODE},
};
use op_alloy_network::Optimism;
use op_alloy_rpc_types::OpTransactionRequest;
use reth::{providers::CanonStateSubscriptions as _, rpc::server_types::eth::EthApiError};
use reth_rpc::EthFilter;
use reth_rpc_convert::RpcReceipt;
use reth_rpc_eth_api::{
    EthApiTypes, EthFilterApiServer, RpcBlock, RpcTransaction,
    helpers::{EthBlocks, EthCall, EthState, EthTransactions, FullEthApi},
};
use tokio::sync::broadcast::error::RecvError;
use tokio_stream::{StreamExt, wrappers::BroadcastStream};

use crate::unsealed_block::UnsealedBlock;

/// Max configured timeout for `eth_sendRawTransactionSync`.
const SEND_RAW_TX_SYNC_TIMEOUT: Duration = Duration::from_millis(6_000);

/// `eth_` API that is aware of unsealed state (frags).
#[rpc(server, namespace = "eth")]
pub trait EthApi {
    /// Returns block by number with support for pending blocks (frags).
    #[method(name = "getBlockByNumber")]
    async fn block_by_number(&self, number: BlockNumberOrTag, full: bool) -> RpcResult<Option<RpcBlock<Optimism>>>;

    /// Returns the transaction receipt for a given transaction hash, with support for frags.
    #[method(name = "getTransactionReceipt")]
    async fn get_transaction_receipt(&self, tx_hash: TxHash) -> RpcResult<Option<RpcReceipt<Optimism>>>;

    /// Returns account balance, with support for pending state.
    #[method(name = "getBalance")]
    async fn get_balance(&self, address: Address, block_number: Option<BlockId>) -> RpcResult<U256>;

    /// Returns transaction count for an address, with support for pending state.
    #[method(name = "getTransactionCount")]
    async fn get_transaction_count(&self, address: Address, block_number: Option<BlockId>) -> RpcResult<U256>;

    /// Returns the transaction for a given hash, with support for frags.
    #[method(name = "getTransactionByHash")]
    async fn transaction_by_hash(&self, tx_hash: TxHash) -> RpcResult<Option<RpcTransaction<Optimism>>>;

    /// Sends a raw transaction and waits for inclusion in a frag.
    #[method(name = "sendRawTransactionSync")]
    async fn send_raw_transaction_sync(
        &self,
        transaction: alloy_primitives::Bytes,
        timeout_ms: Option<u64>,
    ) -> RpcResult<RpcReceipt<Optimism>>;

    /// Executes a call with flashblock state support.
    #[method(name = "call")]
    async fn call(
        &self,
        transaction: OpTransactionRequest,
        block_number: Option<BlockId>,
        state_overrides: Option<StateOverride>,
        block_overrides: Option<Box<BlockOverrides>>,
    ) -> RpcResult<alloy_primitives::Bytes>;

    /// Estimates gas with flashblock state support.
    #[method(name = "estimateGas")]
    async fn estimate_gas(
        &self,
        transaction: OpTransactionRequest,
        block_number: Option<BlockId>,
        overrides: Option<StateOverride>,
    ) -> RpcResult<U256>;

    /// Simulates transactions with flashblock state support.
    #[method(name = "simulateV1")]
    async fn simulate_v1(
        &self,
        opts: SimulatePayload<OpTransactionRequest>,
        block_number: Option<BlockId>,
    ) -> RpcResult<Vec<SimulatedBlock<RpcBlock<Optimism>>>>;

    /// Returns logs matching the filter, including pending flashblock logs.
    #[method(name = "getLogs")]
    async fn get_logs(&self, filter: Filter) -> RpcResult<Vec<Log>>;
}

/// Extended `eth_` API with unsealed state support (frags).
#[derive(Debug)]
pub struct EthApi<Eth: EthApiTypes> {
    pub canonical: Eth,
    pub eth_filter: EthFilter<Eth>,
    pub unsealed_block: Arc<ArcSwapOption<UnsealedBlock>>,
    pub unsealed_as_latest: bool,
}

impl<Eth: EthApiTypes> EthApi<Eth> {
    fn use_unsealed_state(&self, number: &impl Tag) -> bool {
        (self.unsealed_as_latest && number.is_latest()) || number.is_pending()
    }
}

#[async_trait]
impl<Eth> EthApiServer for EthApi<Eth>
where
    Eth: EthApiTypes + FullEthApi<NetworkTypes = Optimism> + Send + Sync + 'static,
    ErrorObject<'static>: From<Eth::Error>,
{
    async fn block_by_number(&self, number: BlockNumberOrTag, full: bool) -> RpcResult<Option<RpcBlock<Optimism>>> {
        tracing::debug!(
            message = "rpc::block_by_number",
            block_number = ?number
        );

        if self.use_unsealed_state(&number) {
            // TODO: Implement pending blocks

            EthBlocks::rpc_block(&self.canonical, BlockNumberOrTag::Latest.into(), full).await.map_err(Into::into)
        } else {
            EthBlocks::rpc_block(&self.canonical, number.into(), full).await.map_err(Into::into)
        }
    }

    async fn get_transaction_receipt(&self, tx_hash: TxHash) -> RpcResult<Option<RpcReceipt<Optimism>>> {
        tracing::debug!(
            message = "rpc::get_transaction_receipt",
            tx_hash = %tx_hash
        );

        // First, check canonical chain
        if let Some(canonical_receipt) = EthTransactions::transaction_receipt(&self.canonical, tx_hash).await? {
            return Ok(Some(canonical_receipt));
        }

        // TODO: Implement pending transaction receipts
        if let Some(unsealed_block) = self.unsealed_block.load_full() {
            if let Some(receipt) = unsealed_block.get_transaction_receipt(&tx_hash) {
                todo!("Type conversion")
            }
        }

        Ok(None)
    }

    async fn get_balance(&self, address: Address, block_number: Option<BlockId>) -> RpcResult<U256> {
        tracing::debug!(
            message = "rpc::get_balance",
            address = %address
        );
        let block_id = block_number.unwrap_or_default();
        if self.use_unsealed_state(&block_id) {
            if let Some(unsealed_block) = self.unsealed_block.load_full() {
                if let Some(balance) = unsealed_block.get_balance(address) {
                    return Ok(balance);
                }
            }
        }

        EthState::balance(&self.canonical, address, block_number).await.map_err(Into::into)
    }

    async fn get_transaction_count(&self, address: Address, block_number: Option<BlockId>) -> RpcResult<U256> {
        tracing::debug!(
            message = "rpc::get_transaction_count",
            address = %address,
        );

        let block_id = block_number.unwrap_or_default();

        let mut count =
            EthState::transaction_count(&self.canonical, address, block_number).await.map_err(Into::into)?;

        if self.use_unsealed_state(&block_id) {
            if let Some(unsealed_block) = self.unsealed_block.load_full() {
                let unsealed_count = unsealed_block.get_transaction_count(address);
                count += unsealed_count;
            }
        }

        Ok(count)
    }

    async fn transaction_by_hash(&self, tx_hash: TxHash) -> RpcResult<Option<RpcTransaction<Optimism>>> {
        tracing::debug!(
            message = "rpc::transaction_by_hash",
            tx_hash = %tx_hash
        );

        // Check canonical chain first to avoid race condition where flashblocks
        // state hasn't been cleared yet after canonical block commit
        if let Some(canonical_tx) = EthTransactions::transaction_by_hash(&self.canonical, tx_hash)
            .await?
            .map(|tx| tx.into_transaction(self.canonical.tx_resp_builder()))
            .transpose()?
        {
            return Ok(Some(canonical_tx));
        }

        if let Some(unsealed_block) = self.unsealed_block.load_full() {
            if let Some(tx) = unsealed_block.get_transaction(&tx_hash) {
                return Ok(Some(tx));
            }
        }

        Ok(None)
    }

    async fn send_raw_transaction_sync(
        &self,
        transaction: alloy_primitives::Bytes,
        timeout_ms: Option<u64>,
    ) -> RpcResult<RpcReceipt<Optimism>> {
        tracing::debug!(message = "rpc::send_raw_transaction_sync");

        let timeout = timeout_ms
            .map(|ms| {
                let timeout = Duration::from_millis(ms);
                if timeout > SEND_RAW_TX_SYNC_TIMEOUT {
                    return Err(ErrorObjectOwned::owned(
                        INVALID_PARAMS_CODE,
                        format!("time out too long, timeout: {ms} ms, max: {SEND_RAW_TX_SYNC_TIMEOUT:?}"),
                        None::<()>,
                    ));
                }

                Ok(timeout)
            })
            .transpose()?
            .unwrap_or(SEND_RAW_TX_SYNC_TIMEOUT);

        let tx_hash = match EthTransactions::send_raw_transaction(&self.canonical, transaction).await {
            Ok(hash) => hash,
            Err(e) => return Err(e.into()),
        };

        tracing::debug!(
            message = "rpc::send_raw_transaction_sync::sent_transaction",
            tx_hash = %tx_hash,
            timeout_ms = timeout_ms,
        );

        loop {
            tokio::select! {
                receipt = self.wait_for_frag_receipt(tx_hash) => {
                    if let Some(receipt) = receipt {
                        return Ok(receipt);
                    } else {
                        continue
                    }
                }
                receipt = self.wait_for_canonical_receipt(tx_hash) => {
                        if let Some(receipt) = receipt {
                            return Ok(receipt);
                        } else {
                            continue
                        }
                    }
                _ = tokio::time::sleep(timeout) => {
                    return Err(EthApiError::TransactionConfirmationTimeout {
                        hash: tx_hash,
                        duration: timeout,
                    }.into());
                }
            }
        }
    }

    async fn call(
        &self,
        transaction: OpTransactionRequest,
        block_number: Option<BlockId>,
        state_overrides: Option<StateOverride>,
        block_overrides: Option<Box<BlockOverrides>>,
    ) -> RpcResult<alloy_primitives::Bytes> {
        tracing::debug!(
            message = "rpc::call",
            transaction = ?transaction,
            block_number = ?block_number,
            state_overrides = ?state_overrides,
            block_overrides = ?block_overrides,
        );

        let block_id = block_number.unwrap_or_default();
        let mut pending_overrides = EvmOverrides::default();
        // If the call is to pending block use cached override (if they exist)
        if self.use_unsealed_state(&block_id) &&
            let Some(unsealed_block) = self.unsealed_block.load_full()
        {
            pending_overrides.state = unsealed_block.get_state_overrides();
        }

        // Apply user's overrides on top
        let mut state_overrides_builder = StateOverridesBuilder::new(pending_overrides.state.unwrap_or_default());
        state_overrides_builder = state_overrides_builder.extend(state_overrides.unwrap_or_default());
        let final_overrides = state_overrides_builder.build();

        // Delegate to the underlying eth_api
        EthCall::call(
            &self.canonical,
            transaction,
            Some(block_id),
            EvmOverrides::new(Some(final_overrides), block_overrides),
        )
        .await
        .map_err(Into::into)
    }

    async fn estimate_gas(
        &self,
        transaction: OpTransactionRequest,
        block_number: Option<BlockId>,
        overrides: Option<StateOverride>,
    ) -> RpcResult<U256> {
        tracing::debug!(
            message = "rpc::estimate_gas",
            transaction = ?transaction,
            block_number = ?block_number,
            overrides = ?overrides,
        );

        let block_id = block_number.unwrap_or_default();
        let mut pending_overrides = EvmOverrides::default();
        // If the call is to pending block use cached override (if they exist)
        if self.use_unsealed_state(&block_id) &&
            let Some(unsealed_block) = self.unsealed_block.load_full()
        {
            pending_overrides.state = unsealed_block.get_state_overrides();
        }

        let mut state_overrides_builder = StateOverridesBuilder::new(pending_overrides.state.unwrap_or_default());
        state_overrides_builder = state_overrides_builder.extend(overrides.unwrap_or_default());
        let final_overrides = state_overrides_builder.build();

        EthCall::estimate_gas_at(&self.canonical, transaction, block_id, Some(final_overrides))
            .await
            .map_err(Into::into)
    }

    async fn simulate_v1(
        &self,
        opts: SimulatePayload<OpTransactionRequest>,
        block_number: Option<BlockId>,
    ) -> RpcResult<Vec<SimulatedBlock<RpcBlock<Eth::NetworkTypes>>>> {
        tracing::debug!(
            message = "rpc::simulate_v1",
            block_number = ?block_number,
        );

        let block_id = block_number.unwrap_or_default();
        let mut pending_overrides = EvmOverrides::default();

        // If the call is to pending block use cached override (if they exist)
        if self.use_unsealed_state(&block_id) &&
            let Some(unsealed_block) = self.unsealed_block.load_full()
        {
            pending_overrides.state = unsealed_block.get_state_overrides();
        }

        // Prepend flashblocks pending overrides to the block state calls
        let mut block_state_calls: Vec<SimBlock<OpTransactionRequest>> = Vec::new();
        for sim_block in opts.block_state_calls {
            let mut state_overrides_builder =
                StateOverridesBuilder::new(pending_overrides.state.clone().unwrap_or_default());
            state_overrides_builder = state_overrides_builder.extend(sim_block.state_overrides.unwrap_or_default());
            let final_overrides = state_overrides_builder.build();

            let block_state_call = SimBlock { state_overrides: Some(final_overrides), ..sim_block };
            block_state_calls.push(block_state_call);
        }

        let payload = SimulatePayload { block_state_calls, ..opts };

        EthCall::simulate_v1(&self.canonical, payload, Some(block_id)).await.map_err(Into::into)
    }

    async fn get_logs(&self, filter: Filter) -> RpcResult<Vec<Log>> {
        tracing::debug!(
            message = "rpc::get_logs",
            address = ?filter.address
        );

        // Check if this is a mixed query (toBlock is pending)
        let (from_block, to_block) = match &filter.block_option {
            FilterBlockOption::Range { from_block, to_block } => (*from_block, *to_block),
            _ => {
                // Block hash queries or other formats - delegate to eth API
                return self.eth_filter.logs(filter).await;
            }
        };

        // If toBlock is not pending, delegate to eth API
        if to_block.is_some_and(|block| !self.use_unsealed_state(&block)) {
            return self.eth_filter.logs(filter).await;
        }

        // Mixed query: toBlock is pending, so we need to combine historical + pending logs
        let mut all_logs = Vec::new();

        if self.use_unsealed_state(&to_block.unwrap_or_default()) &&
            let Some(unsealed_block) = self.unsealed_block.load_full()
        {
            let pending_logs = unsealed_block.get_unsealed_logs(&filter);
            all_logs.extend(pending_logs);
        }

        // Get historical logs if fromBlock is not pending
        if !matches!(from_block, Some(BlockNumberOrTag::Pending)) {
            // Create a filter for historical data (fromBlock to latest)
            let mut historical_filter = filter.clone();
            historical_filter.block_option =
                FilterBlockOption::Range { from_block, to_block: Some(BlockNumberOrTag::Latest) };

            let historical_logs: Vec<Log> = self.eth_filter.logs(historical_filter).await?;
            all_logs.extend(historical_logs);
        }

        // Always get pending logs when toBlock is pending

        // TODO:
        // Dedup any logs from the pending state that may already have been covered in the historical logs
        all_logs.dedup();

        Ok(all_logs)
    }
}

impl<Eth> EthApi<Eth>
where
    Eth: FullEthApi<NetworkTypes = Optimism> + Send + Sync + 'static,
{
    async fn wait_for_frag_receipt(&self, tx_hash: TxHash) -> Option<RpcReceipt<Optimism>> {
        if let Some(unsealed_block) = self.unsealed_block.load_full() {
            let mut receiver = unsealed_block.subscribe_new_blocks();

            loop {
                match receiver.recv().await {
                    Ok(block) => {
                        if let Some(receipt) = unsealed_block.get_transaction_receipt(&tx_hash) {
                            tracing::debug!(%tx_hash, block_number = block.number(), block_hash = %block.hash(), "Receipt found");
                            todo!("Type conversion")
                        }

                        continue;
                    }
                    Err(RecvError::Closed) => {
                        tracing::debug!("Unsealed block receipt queue closed");
                        return None;
                    }
                    Err(RecvError::Lagged(_)) => {
                        tracing::warn!("Unsealed block receipt queue lagged, maybe missing receipts");
                    }
                }
            }
        }

        None
    }

    async fn wait_for_canonical_receipt(&self, tx_hash: TxHash) -> Option<RpcReceipt<Optimism>> {
        let mut stream = BroadcastStream::new(self.canonical.provider().subscribe_to_canonical_state());

        while let Some(Ok(canon_state)) = stream.next().await {
            for (block_receipt, _) in canon_state.block_receipts() {
                for (canonical_tx_hash, _) in &block_receipt.tx_receipts {
                    if *canonical_tx_hash == tx_hash {
                        tracing::debug!(
                            message = "found receipt in canonical state",
                            tx_hash = %tx_hash
                        );
                        return EthTransactions::transaction_receipt(&self.canonical, tx_hash).await.ok().flatten();
                    }
                }
            }
        }
        None
    }
}

/// Helper trait for checking if a block number or id is latest or pending.
trait Tag {
    fn is_latest(&self) -> bool;
    fn is_pending(&self) -> bool;
}

impl Tag for BlockNumberOrTag {
    fn is_latest(&self) -> bool {
        self.is_latest()
    }

    fn is_pending(&self) -> bool {
        self.is_pending()
    }
}

impl Tag for BlockId {
    fn is_latest(&self) -> bool {
        self.is_latest()
    }

    fn is_pending(&self) -> bool {
        self.is_pending()
    }
}
