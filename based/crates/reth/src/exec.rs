use std::future::Future;
use std::sync::Arc;
use alloy_consensus::{BlockBody, Header, Transaction, TxEnvelope};
use alloy_consensus::transaction::{Recovered, SignerRecoverable, TransactionMeta};
use alloy_eips::BlockNumberOrTag;
use alloy_primitives::{B256, BlockNumber, Bytes, Sealable};
use alloy_primitives::utils::ParseUnits::U256;
use alloy_rpc_types::{Block, Log, TransactionReceipt, engine::{ExecutionPayloadV1, ExecutionPayloadV2, ExecutionPayloadV3}};
use reth_chainspec::{ChainSpecProvider, EthChainSpec};
use reth_optimism_chainspec::OpHardforks;
use reth_optimism_evm::{OpEvmConfig, OpNextBlockEnvAttributes};
use reth_revm::database::StateProviderDatabase;
use reth_revm::{DatabaseCommit, State};
use reth_storage_api::{BlockReaderIdExt, StateProviderFactory};
use revm::database::CacheDB;
use arc_swap::ArcSwapOption;
use reth_optimism_primitives::{OpBlock, OpTransactionSigned};
use reth_optimism_primitives::serde_bincode_compat::transaction::OpTxEnvelope;
use alloy_eips::eip2718::Decodable2718;
use alloy_rpc_types::state::StateOverride;
use bop_common::p2p::{EnvV0, FragV0};
use log::error;
use reth_evm::{ConfigureEvm, Evm};
use reth_revm::context::result::ResultAndState;
use crate::{error::ExecError, unsealed_block::UnsealedBlock};

#[derive(Debug, Clone)]
pub struct ExecOutput {
    pub receipts: Vec<TransactionReceipt>,
    pub logs: Vec<Log>,
    pub gas_used_delta: u64,
}

/// This trait is the ONLY place that needs to know about Reth internals.
/// Everything else is just state-machine + bookkeeping.
pub trait UnsealedExecutor: Send {
    /// Ensure the executor context is ready for this env (initialize overlay state, block env, etc.)
    fn ensure_env(&mut self, env: &EnvV0) -> impl Future<Output = Result<(), ExecError>> + Send + '_;

    /// Execute all txs in `frag` on top of current overlay state.
    ///
    /// MUST be cumulative: txs execute after all previous frags's txs.
    fn execute_frag(
        &mut self,
        ub: &UnsealedBlock,
        frag: &FragV0,
    ) -> impl Future<Output = Result<ExecOutput, ExecError>> + Send + '_;

    fn seal(&mut self, ub: &UnsealedBlock) -> impl Future<Output = Result<(), ExecError>> + Send + '_;

    fn set_canonical(&mut self, b: &Block) -> impl Future<Output = Result<(), ExecError>> + Send + '_;

    fn get_block(&self, hash: B256, number: BlockNumber) -> impl Future<Output = Result<Block, ExecError>> + Send + '_;

    /// Reset overlay state completely.
    fn reset(&mut self);
}

/// Apply the executor output to the UnsealedBlock (common logic).
pub fn apply_exec_output(ub: &mut UnsealedBlock, out: ExecOutput) {
    ub.receipts.extend(out.receipts);
    ub.logs.extend(out.logs);
    ub.cumulative_gas_used = ub.cumulative_gas_used.saturating_add(out.gas_used_delta);
}

pub struct StateExecutor <Client> {
    client: Client,
    current_unsealed_block: Arc<ArcSwapOption<UnsealedBlock>>,
}

impl<Client> UnsealedExecutor for StateExecutor<Client> where
    Client: StateProviderFactory
    + ChainSpecProvider<ChainSpec: EthChainSpec<Header = Header> + OpHardforks>
    + BlockReaderIdExt<Header = Header>
    + Clone
    + 'static, {

    fn ensure_env(&mut self, _env: &EnvV0) -> impl Future<Output = Result<(), ExecError>> + Send + '_ {
        async move { Ok(()) }
    }

    fn execute_frag(
        &mut self,
        _ub: &UnsealedBlock,
        frag: &FragV0,
    ) -> impl Future<Output = Result<ExecOutput, ExecError>> + Send + '_ {
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

            let state_provider = client
                .state_by_block_number_or_tag(BlockNumberOrTag::Number(canonical_block))
                .map_err(|e| ExecError::Failed(format!("state_by_block_number_or_tag({canonical_block}) failed: {e}")))?;

            let state_provider_db = StateProviderDatabase::new(state_provider);
            let state = State::builder()
                .with_database(state_provider_db)
                .with_bundle_update()
                .build();

            let mut db = CacheDB { cache: ub_cache, db: state };

            let mut state_overrides = match ub.get_state_overrides() {
                Some(v) => v,
                None => StateOverride::default(),
            };

            let block: OpBlock = build_op_block_from_ub_and_frag(ub.as_ref(), frag)?;
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

            let mut gas_used = 0;
            let mut next_log_index = 0;

            for (idx, transaction) in block.body.transactions.iter().enumerate() {
                let tx_hash = transaction.tx_hash();
                let sender = transaction.recover_signer()?;

                let recovered_transaction = Recovered::new_unchecked(transaction.clone(), sender);
                let envelope = recovered_transaction.clone().convert::<OpTransactionSigned>();

                let effective_gas_price = if transaction.is_deposit() {
                    0
                } else {
                    block
                        .base_fee_per_gas
                        .map(|base_fee| {
                            transaction.effective_tip_per_gas(base_fee).unwrap_or_default()
                                + base_fee as u128
                        })
                        .unwrap_or_else(|| transaction.max_fee_per_gas())
                };

                match evm.transact(recovered_transaction) {
                    Ok(ResultAndState { state, result }) => {
                        for (addr, acc) in &state {
                            let existing_override = state_overrides.entry(*addr).or_default();
                            existing_override.balance = Some(acc.info.balance);
                            existing_override.nonce = Some(acc.info.nonce);
                            existing_override.code =
                                acc.info.code.clone().map(|code| code.bytes());

                            let existing =
                                existing_override.state_diff.get_or_insert(Default::default());
                            let changed_slots = acc.storage.iter().map(|(&key, slot)| {
                                (B256::from(key), B256::from(slot.present_value))
                            });

                            existing.extend(changed_slots);
                        }
                        result.
                        evm.db_mut().commit(state);
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

            Ok(ExecOutput { receipts: vec![], logs: vec![], gas_used_delta: 0 })
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


fn build_op_block_from_ub_and_frag(ub: &UnsealedBlock, frag: FragV0) -> Result<OpBlock, ExecError> {
    // Decode EIP-2718 tx bytes -> OpTxEnvelope
    let tx_list: Vec<OpTransactionSigned> = frag
        .txs
        .iter()
        .enumerate()
        .map(|(i, tx_bytes)| {
            let mut slice = tx_bytes.as_ref(); // &[u8]

            let eth_env = TxEnvelope::decode_2718_exact(&mut slice)
                .map_err(|e| ExecError::Failed(format!("decode tx failed: {e}")))?;

            let op_env = OpTransactionSigned::try_from_eth_envelope(eth_env).map_err(|unsupported| {
                ExecError::Failed(format!(
                    "tx variant not supported on OP (likely EIP-4844): {unsupported:?}"
                ))
            })?;

            Ok(op_env)
        })
        .collect::<Result<Vec<_>, ExecError>>()?;

    // Convert EnvV0.extra_data (VariableList<u8, _>) -> Bytes for the header.
    let extra_data: Bytes = Bytes::copy_from_slice(ub.env.extra_data.as_ref());

    // Like your Go code, these roots/bloom are left "empty"/default because this is
    // a synthetic block used for execution context + bookkeeping.
    let header = Header {
        parent_hash: ub.env.parent_hash,

        // Go: UncleHash = EmptyUncleHash (we'll keep defaults unless you have constants)
        ommers_hash: Default::default(),

        beneficiary: ub.env.beneficiary,

        // Go: Root/TxHash/ReceiptHash/Bloom are empty in InsertNewFrag
        state_root: B256::ZERO,
        transactions_root: B256::ZERO,
        receipts_root: B256::ZERO,
        logs_bloom: Default::default(),

        difficulty: ub.env.difficulty,
        number: frag.block_number,
        gas_limit: ub.env.gas_limit,

        // Go: GasUsed = currentUnsealedBlock.CumulativeGasUsed
        gas_used: ub.cumulative_gas_used,

        timestamp: ub.env.timestamp,
        extra_data,

        // Go: MixDigest = Prevrandao
        mix_hash: ub.env.prevrandao,

        nonce: Default::default(),

        // Go: BaseFee = currentUnsealedBlock.Env.Basefee
        base_fee_per_gas: Some(ub.env.basefee),

        // If you want to mirror Go’s “empty withdrawals list”, set `withdrawals: Some(vec![])`
        // and set withdrawals_root accordingly (if you have the empty-withdrawals root constant).
        withdrawals_root: None,

        // Go: BlobGasUsed = &currentUnsealedBlock.CumulativeBlobGasUsed
        blob_gas_used: Some(ub.cumulative_blob_gas_used),

        // Go: ExcessBlobGas = new(uint64) (i.e. 0)
        excess_blob_gas: Some(0),

        parent_beacon_block_root: Some(ub.env.parent_beacon_block_root),

        // post-requests fields
        requests_hash: None,
    };

    let body = BlockBody {
        transactions: tx_list,
        ommers: vec![],
        withdrawals: None, // or Some(vec![]) if you want an explicit empty list
    };

    Ok(OpBlock::new(header, body))
}