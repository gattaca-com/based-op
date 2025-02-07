use std::{fmt::Display, sync::Arc};

use alloy_consensus::Header;
use alloy_rpc_types::engine::ForkchoiceState;
use bop_common::{
    communication::{
        messages::{
            EvmBlockParams, NextBlockAttributes, SimulatorToSequencer, SimulatorToSequencerMsg, TopOfBlockResult,
        },
        SendersSpine, TrackedSenders,
    },
    db::{state::ensure_create2_deployer, DBFrag, State},
    time::Instant,
    transaction::SimulatedTx,
};
use bop_db::DatabaseRead;
use bop_pool::transaction::pool::TxPool;
use op_alloy_rpc_types_engine::OpPayloadAttributes;
use reth_chainspec::EthereumHardforks;
use reth_evm::{
    env::EvmEnv,
    execute::{BlockExecutionError, BlockValidationError, ProviderError},
    system_calls::SystemCaller,
    ConfigureEvm, ConfigureEvmEnv,
};
use reth_optimism_chainspec::OpChainSpec;
use reth_optimism_evm::{OpBlockExecutionError, OpEvmConfig};
use reth_optimism_forks::{OpHardfork, OpHardforks};
use revm::{
    db::{states::bundle_state::BundleRetention, BundleState },
    Database, DatabaseCommit, Evm,
};
use revm_primitives::{Address, BlockEnv, Bytes, EnvWithHandlerCfg, EvmState, B256};

use crate::{
    block_sync::BlockSync,
    sorting::{ActiveOrders, SortingData},
    FragSequence, SequencerConfig,
};

pub struct SequencerContext<Db> {
    pub config: SequencerConfig,
    pub db: Db,
    pub tx_pool: TxPool,
    pub block_env: BlockEnv,
    pub frags: FragSequence<Db>,
    pub block_executor: BlockSync,
    pub parent_hash: B256,
    pub parent_header: Header,
    pub fork_choice_state: ForkchoiceState,
    pub payload_attributes: Box<OpPayloadAttributes>,
    pub system_caller: SystemCaller<OpEvmConfig, OpChainSpec>,
}

impl<Db: DatabaseRead> SequencerContext<Db> {
    pub fn new(db: Db, db_frag: DBFrag<Db>, config: SequencerConfig) -> Self {
        let frags = FragSequence::new(db_frag, 0);
        let block_executor = BlockSync::new(config.evm_config.chain_spec().clone());
        let system_caller = SystemCaller::new(config.evm_config.clone(), config.evm_config.chain_spec().clone());
        Self {
            db,
            frags,
            block_executor,
            config,
            system_caller,
            tx_pool: Default::default(),
            fork_choice_state: Default::default(),
            payload_attributes: Default::default(),
            parent_hash: Default::default(),
            parent_header: Default::default(),
            block_env: Default::default(),
        }
    }
}
impl<Db> SequencerContext<Db> {
    pub fn chain_spec(&self) -> &Arc<OpChainSpec> {
        self.config.evm_config.chain_spec()
    }

    pub fn extra_data(&self) -> Bytes {
        let timestamp = self.payload_attributes.payload_attributes.timestamp;
        if self.chain_spec().is_holocene_active_at_timestamp(timestamp) {
            self.payload_attributes
                .get_holocene_extra_data(self.chain_spec().base_fee_params_at_timestamp(timestamp))
                .expect("couldn't get extra data")
        } else {
            Bytes::default()
        }
    }
}
impl<Db: Clone> SequencerContext<Db> {
    pub fn new_sorting_data(&self) -> SortingData<Db> {
        SortingData {
            frag: self.frags.create_in_sort(),
            until: Instant::now() + self.config.frag_duration,
            in_flight_sims: 0,
            next_to_be_applied: None,
            tof_snapshot: ActiveOrders::new(self.tx_pool.clone_active()),
        }
    }

    pub fn reset_fragdb(&mut self) {
        self.frags.reset_fragdb(self.db.clone());
    }
}

impl<Db: DatabaseRead + Database<Error: Into<ProviderError> + Display>> SequencerContext<Db> {
    /// Processes a new block from the sequencer by:
    /// 1. Updating EVM environments
    /// 2. Applying pre-execution changes
    /// 3. Processing forced inclusion transactions
    fn on_new_block(&mut self, evm_block_params: EvmBlockParams<Db>) {
        let env_with_handler_cfg = self.get_env_for_new_block(&evm_block_params);
        let (forced_inclusion_txs, state, changes) = self
            .get_start_state_for_new_block(evm_block_params.db, env_with_handler_cfg, &evm_block_params.attributes)
            .expect("shouldn't fail");


    }

    /// Must be called each new block.
    /// Applies pre-execution changes and must include txs from the payload attributes.
    ///
    /// Returns the end state and SimulatedTxs for all must include txs.
    fn get_start_state_for_new_block(
        &mut self,
        db: DBFrag<Db>,
        env_with_handler_cfg: EnvWithHandlerCfg,
        next_attributes: &NextBlockAttributes,
    ) -> Result<(Vec<SimulatedTx>, BundleState, Vec<EvmState>), BlockExecutionError>
    where
        Db: DatabaseRead + Database<Error: Into<ProviderError> + Display>,
    {
        let evm_config = self.config.evm_config.clone();
        let regolith_active = self
            .config
            .evm_config
            .chain_spec()
            .fork(OpHardfork::Regolith)
            .active_at_timestamp(u64::try_from(env_with_handler_cfg.block.timestamp).unwrap());

        // Configure new EVM to apply pre-execution and must include txs.
        let mut state = State::new(db);
        let mut evm = evm_config.evm_with_env(&mut state, env_with_handler_cfg);

        // Apply pre-execution changes.
        let mut changes = self
            .apply_pre_execution_changes(next_attributes, &mut evm)?
            .map(|changes| vec![changes])
            .unwrap_or_default();

        let mut tx_results = Vec::with_capacity(next_attributes.forced_inclusion_txs.len());
        let block_coinbase = evm.block().coinbase;
        // Apply must include txs.
        for tx in next_attributes.forced_inclusion_txs.iter() {
            // Cache some values pre-simulation.
            let start_balance =
                evm.db_mut().basic(block_coinbase).ok().flatten().map(|a| a.balance).unwrap_or_default();
            let depositor_nonce = (tx.is_deposit() && regolith_active)
                .then(|| evm.db_mut().basic(tx.sender()).ok().flatten().map(|a| a.nonce).unwrap_or_default());

            tx.fill_tx_env(evm.tx_mut());

            // Execute transaction.
            let result_and_state = evm.transact().map_err(move |err| {
                let new_err = err.map_db_err(|e| e.into());
                BlockValidationError::EVM { hash: tx.tx_hash(), error: Box::new(new_err) }
            })?;

            self.system_caller.on_state(&result_and_state.state);
            evm.db_mut().commit(result_and_state.state.clone());
            changes.push(result_and_state.state.clone());
            tx_results.push(SimulatedTx::new(
                tx.clone(),
                result_and_state,
                start_balance,
                block_coinbase,
                depositor_nonce,
            ));
        }
        evm.db_mut().merge_transitions(BundleRetention::Reverts);
        let bundle = evm.db_mut().take_bundle();
        Ok((tx_results, bundle, changes))
    }

    /// Applies required state changes before transaction execution:
    /// - Sets state clear flag based on Spurious Dragon hardfork
    /// - Updates beacon root contract
    /// - Ensures create2deployer deployment at canyon transition
    fn apply_pre_execution_changes(
        &mut self,
        next_attributes: &NextBlockAttributes,
        evm: &mut Evm<'_, (), &mut State<DBFrag<Db>>>,
    ) -> Result<Option<EvmState>, BlockExecutionError> {
        let block_number = u64::try_from(evm.block().number).unwrap();
        let block_timestamp = u64::try_from(evm.block().timestamp).unwrap();

        // Set state clear flag if the block is after the Spurious Dragon hardfork.
        evm.db_mut().set_state_clear_flag(self.chain_spec().is_spurious_dragon_active_at_block(block_number));
        let changes = self.system_caller.apply_beacon_root_contract_call(
            block_timestamp,
            block_number,
            next_attributes.parent_beacon_block_root,
            evm,
        )?;

        ensure_create2_deployer(self.chain_spec().clone(), block_timestamp, evm.db_mut())
            .map_err(|_| OpBlockExecutionError::ForceCreate2DeployerFail)?;

        Ok(changes)
    }

    /// Constructs new block environment configuration from parent header and attributes
    fn get_env_for_new_block(&self, evm_block_params: &EvmBlockParams<Db>) -> EnvWithHandlerCfg {
        let EvmEnv { cfg_env_with_handler_cfg, block_env } = self
            .config
            .evm_config
            .next_cfg_and_block_env(&evm_block_params.parent_header, evm_block_params.attributes.env_attributes)
            .expect("Valid block environment configuration");

        EnvWithHandlerCfg::new_with_cfg_env(cfg_env_with_handler_cfg, block_env, Default::default())
    }
}

impl<Db> AsRef<BlockEnv> for SequencerContext<Db> {
    fn as_ref(&self) -> &BlockEnv {
        &self.block_env
    }
}
