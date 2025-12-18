use std::{future::Future, sync::Arc};

use alloy_consensus::{
    BlockBody, Header, Receipt, TxReceipt,
    transaction::{Recovered, SignerRecoverable, TransactionMeta},
};
use alloy_eips::{BlockNumberOrTag, Typed2718, eip2718::Decodable2718};
use alloy_primitives::{B256, BlockNumber, Bytes, Sealable};
use alloy_rpc_types::{Block, Log, TransactionReceipt, state::StateOverride};
use arc_swap::ArcSwapOption;
use bop_common::p2p::{EnvV0, FragV0};
use op_alloy_consensus::{OpReceiptEnvelope, OpTxEnvelope};
use reth_chainspec::{ChainSpecProvider, EthChainSpec};
use reth_evm::{ConfigureEvm, Evm, op_revm::OpHaltReason};
use reth_optimism_chainspec::OpHardforks;
use reth_optimism_evm::{OpEvmConfig, OpNextBlockEnvAttributes};
use reth_optimism_primitives::{OpBlock, OpPrimitives, OpReceipt, OpTransactionSigned};
use reth_optimism_rpc::OpReceiptBuilder;
use reth_revm::{
    DatabaseCommit, State,
    context::result::{ExecutionResult, ResultAndState},
    database::StateProviderDatabase,
};
use reth_rpc_convert::transaction::ConvertReceiptInput;
use reth_storage_api::{BlockReaderIdExt, StateProviderFactory};
use revm::database::CacheDB;

use crate::{error::ExecError, unsealed_block::UnsealedBlock};

/// This trait is the ONLY place that needs to know about Reth internals.
/// Everything else is just state-machine + bookkeeping.
pub trait UnsealedExecutor: Send {
    /// Ensure the executor context is ready for this env (initialize overlay state, block env, etc.)
    fn ensure_env(&mut self, env: &EnvV0) -> impl Future<Output = Result<(), ExecError>> + Send + '_;

    /// Execute all txs in `frag` on top of current overlay state.
    ///
    /// MUST be cumulative: txs execute after all previous frags's txs.
    fn execute_frag(&mut self, frag: &FragV0) -> impl Future<Output = Result<(), ExecError>> + Send + '_;

    fn seal(&mut self, ub: &UnsealedBlock) -> impl Future<Output = Result<(), ExecError>> + Send + '_;

    fn set_canonical(&mut self, b: &Block) -> impl Future<Output = Result<(), ExecError>> + Send + '_;

    fn get_block(&self, hash: B256, number: BlockNumber) -> impl Future<Output = Result<Block, ExecError>> + Send + '_;

    /// Reset overlay state completely.
    fn reset(&mut self);
}

pub struct StateExecutor<Client> {
    client: Client,
    current_unsealed_block: Arc<ArcSwapOption<UnsealedBlock>>,
}

impl<Client> UnsealedExecutor for StateExecutor<Client>
where
    Client: StateProviderFactory
        + ChainSpecProvider<ChainSpec: EthChainSpec<Header = Header> + OpHardforks>
        + BlockReaderIdExt<Header = Header>
        + Clone
        + 'static,
{
    fn ensure_env(&mut self, _env: &EnvV0) -> impl Future<Output = Result<(), ExecError>> + Send + '_ {
        async move { Ok(()) }
    }

    fn execute_frag(&mut self, frag: &FragV0) -> impl Future<Output = Result<(), ExecError>> + Send + '_ {
        let client = self.client.clone();
        let ub_opt = self.current_unsealed_block.load_full();
        let frag = frag.clone();

        async move {
            let ub = ub_opt.ok_or(ExecError::NotInitialized)?;
            let ub_cache = ub.get_db_cache();
            let canonical_block = ub.env.number.saturating_sub(1);
            let last_block_header = client
                .header_by_number(canonical_block)
                .map_err(|e| ExecError::Failed(format!("header_by_number({canonical_block}) failed: {e}")))?
                .ok_or_else(|| ExecError::Failed(format!("missing parent header at {canonical_block}")))?;

            let evm_config = OpEvmConfig::optimism(client.chain_spec());

            let state_provider =
                client.state_by_block_number_or_tag(BlockNumberOrTag::Number(canonical_block)).map_err(|e| {
                    ExecError::Failed(format!("state_by_block_number_or_tag({canonical_block}) failed: {e}"))
                })?;

            let state_provider_db = StateProviderDatabase::new(state_provider);
            let state = State::builder().with_database(state_provider_db).with_bundle_update().build();

            let mut db = CacheDB { cache: ub_cache, db: state };

            let mut state_overrides = match ub.get_state_overrides() {
                Some(v) => v,
                None => StateOverride::default(),
            };

            let block: OpBlock = build_op_block_from_ub_and_frag(ub.as_ref(), &frag)?;
            let mut l1_block_info = reth_optimism_evm::extract_l1_info(&block.body)?;
            let header = block.header.clone().seal_slow();

            let block_env_attributes = OpNextBlockEnvAttributes {
                timestamp: ub.env.timestamp,
                suggested_fee_recipient: ub.env.beneficiary,
                prev_randao: ub.env.prevrandao,
                gas_limit: ub.env.gas_limit,
                parent_beacon_block_root: Some(ub.env.parent_beacon_block_root),
                extra_data: block.extra_data.clone(),
            };

            let evm_env = evm_config.next_evm_env(&last_block_header, &block_env_attributes)?;
            let mut evm = evm_config.evm_with_env(db, evm_env);

            let mut gas_used: u64 = ub.cumulative_blob_gas_used;
            let mut logs: Vec<Log> = Vec::new();
            let mut next_log_index = 0;
            let mut receipts: Vec<TransactionReceipt<OpReceiptEnvelope<Log>>> = Vec::new();

            for (idx, transaction) in block.body.transactions.iter().enumerate() {
                let tx_hash = transaction.tx_hash();
                let sender = transaction.recover_signer()?;

                let recovered_transaction = Recovered::new_unchecked(transaction.clone(), sender);

                match evm.transact(recovered_transaction) {
                    Ok(ResultAndState { state, result }) => {
                        for (addr, acc) in &state {
                            let existing_override = state_overrides.entry(*addr).or_default();
                            existing_override.balance = Some(acc.info.balance);
                            existing_override.nonce = Some(acc.info.nonce);
                            existing_override.code = acc.info.code.clone().map(|code| code.bytes());

                            let existing = existing_override.state_diff.get_or_insert(Default::default());
                            let changed_slots = acc
                                .storage
                                .iter()
                                .map(|(&key, slot)| (B256::from(key), B256::from(slot.present_value)));

                            existing.extend(changed_slots);
                        }

                        evm.db_mut().commit(state);

                        let (success, tx_gas_used, tx_logs) = split_execution_result(&result);
                        gas_used = gas_used.saturating_add(tx_gas_used);
                        logs.extend(tx_logs.iter().map(|inner| Log { inner: inner.clone(), ..Default::default() }));

                        let base_receipt =
                            Receipt { status: success.into(), cumulative_gas_used: gas_used, logs: tx_logs };

                        let ty = transaction.ty();

                        let op_receipt = wrap_op_receipt(ty, base_receipt, None, None)?;

                        let meta = TransactionMeta {
                            tx_hash,
                            index: idx as u64,
                            block_hash: header.hash(),
                            block_number: block.number,
                            base_fee: block.base_fee_per_gas,
                            excess_blob_gas: block.excess_blob_gas,
                            timestamp: block.timestamp,
                        };

                        let op_cgu = op_receipt.cumulative_gas_used();
                        let input: ConvertReceiptInput<'_, OpPrimitives> = ConvertReceiptInput {
                            receipt: op_receipt,
                            tx: Recovered::new_unchecked(transaction, sender),
                            gas_used: op_cgu,
                            next_log_index,
                            meta,
                        };

                        let receipt =
                            OpReceiptBuilder::new(self.client.chain_spec().as_ref(), input, &mut l1_block_info)?
                                .core_receipt;

                        next_log_index += receipt.logs().len();

                        receipts.push(receipt)
                    }
                    Err(e) => {
                        return Err(ExecError::Failed(format!(
                            "failed to execute transaction: {:?} tx_hash: {:?} sender: {:?}",
                            e, tx_hash, sender
                        )));
                    }
                }
            }

            db = evm.into_db();
            let mut next = ub.clone_for_update().with_db_cache(db.cache).with_state_overrides(Some(state_overrides));
            next.accept_frag_execution(frag, logs, receipts, gas_used);

            self.current_unsealed_block.store(Some(Arc::new(next)));

            Ok(())
        }
    }

    fn seal(&mut self, _ub: &UnsealedBlock) -> impl Future<Output = Result<(), ExecError>> + Send + '_ {
        async move { Ok(()) }
    }

    fn set_canonical(&mut self, _b: &Block) -> impl Future<Output = Result<(), ExecError>> + Send + '_ {
        async move { Ok(()) }
    }

    fn get_block(
        &self,
        _hash: B256,
        _number: BlockNumber,
    ) -> impl Future<Output = Result<Block, ExecError>> + Send + '_ {
        async move { Ok(Block::default()) }
    }

    fn reset(&mut self) {}
}

fn build_op_block_from_ub_and_frag(ub: &UnsealedBlock, frag: &FragV0) -> Result<OpBlock, ExecError> {
    // Decode EIP-2718 tx bytes -> OpTransactionSigned
    let tx_list: Vec<OpTransactionSigned> = frag
        .txs
        .iter()
        .enumerate()
        .map(|(_, tx_bytes)| {
            Ok(OpTxEnvelope::decode_2718(&mut tx_bytes.as_ref())
                .map_err(|e| ExecError::Failed(format!("decode tx failed: {e}")))?)
        })
        .collect::<Result<Vec<_>, ExecError>>()?;

    let extra_data: Bytes = Bytes::copy_from_slice(ub.env.extra_data.as_ref());
    let header = Header {
        parent_hash: ub.env.parent_hash,
        ommers_hash: Default::default(),
        beneficiary: ub.env.beneficiary,
        state_root: B256::ZERO,
        transactions_root: B256::ZERO,
        receipts_root: B256::ZERO,
        logs_bloom: Default::default(),
        difficulty: ub.env.difficulty,
        number: frag.block_number,
        gas_limit: ub.env.gas_limit,
        gas_used: ub.cumulative_gas_used,
        timestamp: ub.env.timestamp,
        extra_data,
        mix_hash: ub.env.prevrandao,
        nonce: Default::default(),
        base_fee_per_gas: Some(ub.env.basefee),
        withdrawals_root: None,
        blob_gas_used: Some(ub.cumulative_blob_gas_used),
        excess_blob_gas: Some(0),
        parent_beacon_block_root: Some(ub.env.parent_beacon_block_root),
        requests_hash: None,
    };

    let body = BlockBody { transactions: tx_list, ommers: vec![], withdrawals: None };

    Ok(OpBlock::new(header, body))
}

fn split_execution_result(result: &ExecutionResult<OpHaltReason>) -> (bool, u64, Vec<alloy_primitives::Log>) {
    match result {
        ExecutionResult::Success { gas_used, logs, .. } => (true, *gas_used, logs.clone()),
        ExecutionResult::Revert { gas_used, .. } => (false, *gas_used, vec![]),
        ExecutionResult::Halt { gas_used, .. } => (false, *gas_used, vec![]),
    }
}

fn wrap_op_receipt(
    tx_type: u8,
    receipt: Receipt<alloy_primitives::Log>,
    deposit_nonce: Option<u64>,
    deposit_receipt_version: Option<u64>,
) -> Result<OpReceipt, ExecError> {
    Ok(match tx_type {
        0x00 => OpReceipt::Legacy(receipt),
        0x01 => OpReceipt::Eip2930(receipt),
        0x02 => OpReceipt::Eip1559(receipt),
        0x04 => OpReceipt::Eip7702(receipt),
        t if t == op_alloy_consensus::DEPOSIT_TX_TYPE_ID => OpReceipt::Deposit(op_alloy_consensus::OpDepositReceipt {
            inner: receipt,
            deposit_nonce,
            deposit_receipt_version,
        }),
        other => return Err(ExecError::Failed(format!("unsupported tx type for receipt: 0x{other:02x}"))),
    })
}
