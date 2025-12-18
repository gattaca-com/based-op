use std::{collections::HashSet, time::Duration};

use alloy_eips::{BlockId, BlockNumberOrTag};
use alloy_primitives::{Address, TxHash, U256};
use alloy_rpc_types::{
    BlockOverrides, Filter, FilterBlockOption, Log,
    simulate::{SimBlock, SimulatePayload, SimulatedBlock},
    state::{EvmOverrides, StateOverride, StateOverridesBuilder},
};
use jsonrpsee::{
    core::{RpcResult, async_trait},
    proc_macros::rpc,
    types::{ErrorObject, ErrorObjectOwned, error::INVALID_PARAMS_CODE},
};
use op_alloy_network::Optimism;
use op_alloy_rpc_types::OpTransactionRequest;
use reth::{providers::CanonStateSubscriptions as _, rpc::server_types::eth::EthApiError};
use reth_rpc::EthFilter;
use reth_rpc_eth_api::{
    EthApiTypes, EthFilterApiServer, RpcBlock, RpcReceipt, RpcTransaction,
    helpers::{EthBlocks, EthCall, EthState, EthTransactions, FullEthApi},
};
use tokio_stream::{StreamExt, wrappers::BroadcastStream};

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
    pub unsealed_state: (),
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

        Ok(None)
    }

    async fn get_balance(&self, address: Address, block_number: Option<BlockId>) -> RpcResult<U256> {
        tracing::debug!(
            message = "rpc::get_balance",
            address = %address
        );
        let block_id = block_number.unwrap_or_default();
        if self.use_unsealed_state(&block_id) {
            // TODO: Pending balance
        }

        EthState::balance(&self.canonical, address, block_number).await.map_err(Into::into)
    }

    async fn get_transaction_count(&self, address: Address, block_number: Option<BlockId>) -> RpcResult<U256> {
        tracing::debug!(
            message = "rpc::get_transaction_count",
            address = %address,
        );

        let block_id = block_number.unwrap_or_default();
        if self.use_unsealed_state(&block_id) {
            todo!();
            // let pending_blocks = self.flashblocks_state.get_pending_blocks();
            // let canon_block = pending_blocks.get_canonical_block_number();
            // let fb_count = pending_blocks.get_transaction_count(address);

            // let fb_count = 0;

            // let canon_count = EthState::transaction_count(&self.canonical, address, Some(canon_block.into()))
            //     .await
            //     .map_err(Into::into)?;

            // return Ok(canon_count + fb_count);
        }

        EthState::transaction_count(&self.canonical, address, block_number).await.map_err(Into::into)
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

        // TODO:
        // Fall back to flashblocks for pending transactions
        // let pending_blocks = self.flashblocks_state.get_pending_blocks();
        // if let Some(fb_transaction) = pending_blocks.get_transaction_by_hash(tx_hash) {
        //     self.metrics.get_transaction_by_hash.increment(1);
        //     return Ok(Some(fb_transaction));
        // }

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

        let mut block_id = block_number.unwrap_or_default();
        let mut pending_overrides = EvmOverrides::default();
        // If the call is to pending block use cached override (if they exist)
        if self.use_unsealed_state(&block_id) {
            // TODO:
            // let pending_blocks = self.flashblocks_state.get_pending_blocks();
            // block_id = pending_blocks.get_canonical_block_number().into();
            // pending_overrides.state = pending_blocks.get_state_overrides();
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

        let mut block_id = block_number.unwrap_or_default();
        let mut pending_overrides = EvmOverrides::default();
        // If the call is to pending block use cached override (if they exist)
        if self.use_unsealed_state(&block_id) {
            // TODO:
            // let pending_blocks = self.flashblocks_state.get_pending_blocks();
            // block_id = pending_blocks.get_canonical_block_number().into();
            // pending_overrides.state = pending_blocks.get_state_overrides();
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

        let mut block_id = block_number.unwrap_or_default();
        let mut pending_overrides = EvmOverrides::default();

        // If the call is to pending block use cached override (if they exist)
        if self.use_unsealed_state(&block_id) {
            // TODO:
            // let pending_blocks = self.flashblocks_state.get_pending_blocks();
            // block_id = pending_blocks.get_canonical_block_number().into();
            // pending_overrides.state = pending_blocks.get_state_overrides();
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

        // TODO:
        // let pending_blocks = self.flashblocks_state.get_pending_blocks();
        // let pending_logs = pending_blocks.get_pending_logs(&filter);

        let mut fetched_logs = HashSet::new();
        // Get historical logs if fromBlock is not pending
        if !matches!(from_block, Some(BlockNumberOrTag::Pending)) {
            // Create a filter for historical data (fromBlock to latest)
            let mut historical_filter = filter.clone();
            historical_filter.block_option =
                FilterBlockOption::Range { from_block, to_block: Some(BlockNumberOrTag::Latest) };

            let historical_logs: Vec<Log> = self.eth_filter.logs(historical_filter).await?;
            for log in &historical_logs {
                fetched_logs.insert((log.block_number, log.log_index));
            }
            all_logs.extend(historical_logs);
        }

        // Always get pending logs when toBlock is pending

        // TODO:
        // Dedup any logs from the pending state that may already have been covered in the historical logs
        // let deduped_pending_logs: Vec<Log> = pending_logs
        //     .iter()
        //     .filter(|log| !fetched_logs.contains(&(log.block_number, log.log_index)))
        //     .cloned()
        //     .collect();
        // all_logs.extend(deduped_pending_logs);

        Ok(all_logs)
    }
}

impl<Eth> EthApi<Eth>
where
    Eth: FullEthApi<NetworkTypes = Optimism> + Send + Sync + 'static,
{
    async fn wait_for_frag_receipt(&self, tx_hash: TxHash) -> Option<RpcReceipt<Optimism>> {
        // TODO: Subscribe to frags
        // let mut receiver = self.flashblocks_state.subscribe_to_flashblocks();

        // loop {
        //     match receiver.recv().await {
        //         Ok(pending_state) if pending_state.get_receipt(tx_hash).is_some() => {
        //             debug!(message = "found receipt in flashblock", tx_hash = %tx_hash);
        //             return pending_state.get_receipt(tx_hash);
        //         }
        //         Ok(_) => {
        //             trace!(message = "flashblock does not contain receipt", tx_hash = %tx_hash);
        //         }
        //         Err(RecvError::Closed) => {
        //             debug!(message = "flashblocks receipt queue closed");
        //             return None;
        //         }
        //         Err(RecvError::Lagged(_)) => {
        //             warn!("Flashblocks receipt queue lagged, maybe missing receipts");
        //         }
        //     }
        // }

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
