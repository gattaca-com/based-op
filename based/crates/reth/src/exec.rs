use std::sync::{Arc, Mutex};

use alloy_consensus::{
    BlockBody, Header, Receipt, Transaction,
    transaction::{Recovered, SignerRecoverable, TransactionMeta},
};
use alloy_eips::{BlockNumberOrTag, Typed2718, eip2718::Decodable2718};
use alloy_primitives::{B256, Bytes, Sealable};
use alloy_rpc_types::Log;
use arc_swap::ArcSwapOption;
use bop_common::{
    p2p::{EnvV0, FragV0},
    typedefs::Database,
};
use op_alloy_consensus::OpTxEnvelope;
use op_alloy_rpc_types::{OpTransactionReceipt, Transaction as RPCTransaction};
use reth::{
    api::Block as RethBlock,
    network::cache::LruMap,
    primitives::{SealedBlock, SealedHeader},
};
use reth_chainspec::{ChainSpecProvider, EthChainSpec, EthereumHardforks as _};
use reth_evm::{ConfigureEvm, Evm, op_revm::OpHaltReason};
use reth_optimism_chainspec::OpHardforks;
use reth_optimism_consensus::isthmus;
use reth_optimism_evm::{OpEvmConfig, OpNextBlockEnvAttributes};
use reth_optimism_primitives::{OpBlock, OpPrimitives, OpReceipt, OpTransactionSigned};
use reth_optimism_rpc::OpReceiptBuilder;
use reth_revm::{
    DatabaseCommit, State,
    context::result::{ExecutionResult, ResultAndState},
    database::StateProviderDatabase,
};
use reth_rpc_convert::transaction::ConvertReceiptInput;
use reth_storage_api::{
    BlockReaderIdExt, BlockWriter, CanonChainTracker, DBProvider, DatabaseProviderFactory, StateProviderFactory,
};
use revm::database::CacheDB;

use crate::{error::ExecError, unsealed_block::UnsealedBlock};

const BLOCK_CACHE_LIMIT: u32 = 256;

/// This trait is the ONLY place that needs to know about Reth internals.
/// Everything else is just state-machine + bookkeeping.
pub trait UnsealedExecutor: Send {
    /// Ensure the executor context is ready for this env (initialize overlay state, block env, etc.)
    fn ensure_env(&mut self, env: &EnvV0) -> Result<(), ExecError>;

    /// Execute all txs in `frag` on top of current overlay state.
    ///
    /// MUST be cumulative: txs execute after all previous frags's txs.
    fn execute_frag(&mut self, frag: &FragV0) -> Result<(), ExecError>;

    fn seal(&mut self) -> Result<(), ExecError>;

    fn set_canonical(&mut self, b: &OpBlock) -> Result<(), ExecError>;

    fn get_block(&self, hash: B256) -> Result<OpBlock, ExecError>;

    /// Reset overlay state completely.
    fn reset(&mut self);
}

pub struct StateExecutor<Client> {
    client: Client,
    current_unsealed_block: Arc<ArcSwapOption<UnsealedBlock>>,
    block_cache: Mutex<LruMap<B256, OpBlock>>,
}

impl<Client> StateExecutor<Client> {
    pub fn new(client: Client) -> Self {
        Self {
            client,
            current_unsealed_block: Arc::new(ArcSwapOption::new(None)),
            block_cache: Mutex::new(LruMap::new(BLOCK_CACHE_LIMIT)),
        }
    }

    pub fn shared_unsealed_block(&self) -> Arc<ArcSwapOption<UnsealedBlock>> {
        Arc::clone(&self.current_unsealed_block)
    }
}

impl<Client> UnsealedExecutor for StateExecutor<Client>
where
    Client: StateProviderFactory
        + ChainSpecProvider<ChainSpec: EthChainSpec<Header = Header> + OpHardforks>
        + BlockReaderIdExt<Header = Header, Block = OpBlock>
        + CanonChainTracker<Header = Header>
        + DatabaseProviderFactory
        + Clone
        + 'static,
    <Client as DatabaseProviderFactory>::ProviderRW: BlockWriter<Block = OpBlock>,
{
    fn ensure_env(&mut self, env: &EnvV0) -> Result<(), ExecError> {
        let Some(parent) = self.client.block_by_hash(env.parent_hash)? else {
            return Err(ExecError::Failed(format!("parent block {} not found", env.parent_hash)))
        };

        let parent_header = parent.header();
        let None = self.current_unsealed_block.load_full() else { return Err(ExecError::NotInitialized) };

        let expected_block_number = parent_header.number.saturating_sub(1);
        if env.number != expected_block_number {
            return Err(ExecError::Failed(format!(
                "env block number doesn't match expected block number, expected {}, received {}",
                expected_block_number, env.number
            )))
        }

        if env.timestamp < parent_header.timestamp {
            return Err(ExecError::Failed(format!(
                "env timestamp is lower than parent block timestamp, parent timestamp {}, env timestamp {}",
                parent_header.timestamp, env.timestamp
            )))
        }

        let state_provider =
            self.client.state_by_block_number_or_tag(BlockNumberOrTag::Number(parent_header.number))?;
        let state_provider_db = StateProviderDatabase::new(state_provider);
        let state = State::builder().with_database(state_provider_db).with_bundle_update().build();

        // Check if the current block is a prague block
        let is_prague = self.client.chain_spec().is_prague_active_at_timestamp(env.timestamp);

        let ub = UnsealedBlock::new(env.clone(), is_prague).with_db_cache(CacheDB::new(state).cache);
        self.current_unsealed_block.store(Some(Arc::new(ub)));

        Ok(())
    }

    fn execute_frag(&mut self, frag: &FragV0) -> Result<(), ExecError> {
        let chain_spec = self.client.chain_spec().clone();

        let ub_arc_opt = self.current_unsealed_block.load_full();
        let frag = frag.clone();

        let ub_arc = ub_arc_opt.ok_or(ExecError::NotInitialized)?;

        // Make an owned, mutable working copy from the start
        let mut ub = ub_arc.as_ref().clone_for_update();

        let ub_cache = ub.get_db_cache();
        let canonical_block = ub.env.number.saturating_sub(1);

        let last_block_header = self
            .client
            .header_by_number(canonical_block)
            .map_err(|e| ExecError::Failed(format!("header_by_number({canonical_block}) failed: {e}")))?
            .ok_or_else(|| ExecError::Failed(format!("missing parent header at {canonical_block}")))?;

        let evm_config = OpEvmConfig::optimism(self.client.chain_spec());

        let state_provider = self
            .client
            .state_by_block_number_or_tag(BlockNumberOrTag::Number(canonical_block))
            .map_err(|e| ExecError::Failed(format!("state_by_block_number_or_tag({canonical_block}) failed: {e}")))?;

        let state_provider_db = StateProviderDatabase::new(state_provider);
        let state = State::builder().with_database(state_provider_db).with_bundle_update().build();

        let mut db = CacheDB { cache: ub_cache, db: state };

        let mut state_overrides = ub.get_state_overrides().unwrap_or_default();

        let block: OpBlock = build_op_block_from_ub_and_frag(&ub, &frag)?;
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
        let mut next_log_index = 0usize;
        let mut receipts: Vec<OpTransactionReceipt> = Vec::new();

        for (idx, transaction) in block.body.transactions.iter().enumerate() {
            let tx_hash = transaction.tx_hash();
            let sender = transaction.recover_signer()?;
            ub.increment_nonce(sender);

            let recovered_transaction = Recovered::new_unchecked(transaction.clone(), sender);
            let envelope = recovered_transaction.clone().convert::<OpTxEnvelope>();
            let is_deposit = transaction.is_deposit();

            let effective_gas_price = if is_deposit {
                0
            } else {
                block
                    .base_fee_per_gas
                    .map(|base_fee| transaction.effective_tip_per_gas(base_fee).unwrap_or_default() + base_fee as u128)
                    .unwrap_or_else(|| transaction.max_fee_per_gas())
            };

            let deposit_nonce = if is_deposit && chain_spec.is_regolith_active_at_timestamp(ub.env.timestamp) {
                // depositor nonce (use signer account)
                let acc = evm
                    .db_mut()
                    .basic(sender)
                    .map_err(|e| ExecError::Failed(format!("get acc nonce basic() failed: {e}")))?
                    .unwrap_or_default();
                Some(acc.nonce) // pre-tx nonce
            } else {
                None
            };

            let deposit_receipt_version =
                if is_deposit && chain_spec.is_canyon_active_at_timestamp(ub.env.timestamp) { Some(1) } else { None };

            let rpc_txn = RPCTransaction {
                inner: alloy_rpc_types_eth::Transaction {
                    inner: envelope,
                    block_hash: Some(header.hash()),
                    block_number: Some(block.number),
                    transaction_index: Some(idx as u64),
                    effective_gas_price: Some(effective_gas_price),
                },
                deposit_nonce,
                deposit_receipt_version,
            };

            ub.with_transaction(rpc_txn);

            match evm.transact(recovered_transaction) {
                Ok(ResultAndState { state, result }) => {
                    for (addr, acc) in &state {
                        let existing_override = state_overrides.entry(*addr).or_default();
                        existing_override.balance = Some(acc.info.balance);
                        existing_override.nonce = Some(acc.info.nonce);
                        existing_override.code = acc.info.code.clone().map(|code| code.bytes());

                        let existing = existing_override.state_diff.get_or_insert(Default::default());
                        let changed_slots =
                            acc.storage.iter().map(|(&key, slot)| (B256::from(key), B256::from(slot.present_value)));

                        existing.extend(changed_slots);
                    }

                    evm.db_mut().commit(state);

                    let (success, tx_gas_used, tx_logs) = split_execution_result(&result);
                    gas_used = gas_used.saturating_add(tx_gas_used);

                    logs.extend(tx_logs.iter().map(|inner| Log { inner: inner.clone(), ..Default::default() }));

                    let base_receipt = Receipt { status: success.into(), cumulative_gas_used: gas_used, logs: tx_logs };

                    let ty = transaction.ty();
                    let op_receipt = wrap_op_receipt(ty, base_receipt, deposit_nonce, deposit_receipt_version)?;

                    let meta = TransactionMeta {
                        tx_hash,
                        index: idx as u64,
                        block_hash: header.hash(),
                        block_number: block.number,
                        base_fee: block.base_fee_per_gas,
                        excess_blob_gas: block.excess_blob_gas,
                        timestamp: block.timestamp,
                    };

                    let input: ConvertReceiptInput<'_, OpPrimitives> = ConvertReceiptInput {
                        receipt: op_receipt,
                        tx: Recovered::new_unchecked(transaction, sender),
                        gas_used: tx_gas_used,
                        next_log_index,
                        meta,
                    };

                    let receipt = OpReceiptBuilder::new(chain_spec.as_ref(), input, &mut l1_block_info)?.build();

                    // TODO: Is this correct?q
                    next_log_index += receipt.inner.logs().len();
                    ub.with_transaction_receipt(tx_hash, receipt.clone());
                    receipts.push(receipt);
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
        ub = ub
            .with_db_cache(db.cache)
            .with_state_overrides(Some(state_overrides))
            .with_bundle_state(db.db.bundle_state);

        ub.accept_frag_execution(frag, logs, receipts, gas_used);

        self.current_unsealed_block.store(Some(Arc::new(ub)));

        Ok(())
    }

    fn seal(&mut self) -> Result<(), ExecError> {
        let ub = self.current_unsealed_block.load_full().ok_or(ExecError::NotInitialized)?;
        let withdrawals_hash = if ub.is_prague {
            let canonical_block = ub.env.number.saturating_sub(1);

            let state_provider =
                self.client.state_by_block_number_or_tag(BlockNumberOrTag::Number(canonical_block)).map_err(|e| {
                    ExecError::Failed(format!("state_by_block_number_or_tag({canonical_block}) failed: {e}"))
                })?;
            let bundle_state = ub.get_bundle_state();
            Some(isthmus::withdrawals_root(bundle_state, state_provider)?)
        } else {
            None
        };

        let block = ub.to_op_block(withdrawals_hash)?;
        let sealed = SealedBlock::seal_slow(block);
        let recovered = sealed.try_recover().map_err(|e| ExecError::Failed(format!("recover senders: {e}")))?;

        let provider_rw = self.client.database_provider_rw()?;
        provider_rw.insert_block(recovered)?;
        provider_rw.commit()?;
        Ok(())
    }

    fn set_canonical(&mut self, b: &OpBlock) -> Result<(), ExecError> {
        let sealed = SealedHeader::seal_slow(b.header.clone());
        self.client.set_canonical_head(sealed);
        Ok(())
    }

    fn get_block(&self, hash: B256) -> Result<OpBlock, ExecError> {
        if let Some(block) = self
            .block_cache
            .lock()
            .map_err(|_| ExecError::Failed("block_cache mutex poisoned".into()))?
            .get(&hash)
            .cloned()
        {
            return Ok(block);
        }

        // fetch
        let block = self
            .client
            .block_by_hash(hash)
            .map_err(|e| ExecError::Failed(format!("block_by_hash failed: {e}")))?
            .ok_or_else(|| ExecError::Failed("pre-sealed block not found".into()))?;

        self.block_cache
            .lock()
            .map_err(|_| ExecError::Failed("block_cache mutex poisoned".into()))?
            .insert(hash, block.clone());

        Ok(block)
    }

    fn reset(&mut self) {
        self.current_unsealed_block.store(None);
    }
}

fn build_op_block_from_ub_and_frag(ub: &UnsealedBlock, frag: &FragV0) -> Result<OpBlock, ExecError> {
    // Decode EIP-2718 tx bytes -> OpTransactionSigned
    let tx_list: Vec<OpTransactionSigned> = frag
        .txs
        .iter()
        .map(|tx_bytes| {
            OpTxEnvelope::decode_2718(&mut tx_bytes.as_ref())
                .map_err(|e| ExecError::Failed(format!("decode tx failed: {e}")))
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
