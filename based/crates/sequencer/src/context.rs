use std::{fmt::Display, sync::Arc};

use alloy_consensus::Header;
use alloy_eips::eip4788::BEACON_ROOTS_ADDRESS;
use alloy_rpc_types::engine::ForkchoiceState;
use bop_common::{
    communication::{
        messages::{EvmBlockParams, NextBlockAttributes, SimulatorToSequencer, SimulatorToSequencerMsg},
        SendersSpine, TrackedSenders,
    },
    db::{state::ensure_create2_deployer, DBFrag, State},
    time::Instant,
    transaction::{SimulatedTx, Transaction},
};
use bop_db::DatabaseRead;
use bop_pool::transaction::pool::TxPool;
use op_alloy_rpc_types_engine::OpPayloadAttributes;
use reth_chainspec::EthereumHardforks;
use reth_evm::{
    env::EvmEnv,
    execute::{BlockExecutionError, BlockValidationError, ProviderError},
    system_calls::SystemCaller,
    ConfigureEvm, ConfigureEvmEnv, NextBlockEnvAttributes,
};
use reth_optimism_chainspec::OpChainSpec;
use reth_optimism_evm::{OpBlockExecutionError, OpEvmConfig};
use reth_optimism_forks::{OpHardfork, OpHardforks};
use revm::{
    db::{states::bundle_state::BundleRetention, BundleState},
    Database, DatabaseCommit, Evm,
};
use revm_primitives::{Address, BlockEnv, Bytes, EnvWithHandlerCfg, EvmState, B256};

use crate::{
    block_sync::BlockSync, simulator::simulate_tx_inner, sorting::{ActiveOrders, SortingData}, FragSequence, SequencerConfig
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
}

impl<Db: DatabaseRead + Database<Error: Into<ProviderError> + Display>> SequencerContext<Db> {
    /// Processes a new block from the sequencer by:
    /// 1. Updating EVM environments
    /// 2. Applying pre-execution changes
    /// 3. Processing forced inclusion transactions
    pub fn on_new_block(&mut self, attributes: Box<OpPayloadAttributes>, senders: &SendersSpine<Db>) {
        self.payload_attributes = attributes;
        let forced_inclusion_txs = self.get_start_state_for_new_block(senders).expect("shouldn't fail");
        self.tx_pool.remove_mined_txs(forced_inclusion_txs.iter(), self.block_env.basefee.to());
        self.frags.reset(self.payload_attributes.gas_limit.unwrap(), forced_inclusion_txs);
    }

    /// Must be called each new block.
    /// Applies pre-execution changes and must include txs from the payload attributes.
    ///
    /// Returns the end state and SimulatedTxs for all must include txs.
    fn get_start_state_for_new_block(
        &mut self,
        senders: &SendersSpine<Db>,
    ) -> Result<Vec<SimulatedTx>, BlockExecutionError>
    where
        Db: DatabaseRead + Database<Error: Into<ProviderError> + Display>,
    {
        let gas_limit = self.payload_attributes.gas_limit.unwrap();
        let env_attributes = NextBlockEnvAttributes {
            timestamp: self.payload_attributes.payload_attributes.timestamp,
            suggested_fee_recipient: self.payload_attributes.payload_attributes.suggested_fee_recipient,
            prev_randao: self.payload_attributes.payload_attributes.prev_randao,
            gas_limit,
        };
        let EvmEnv { cfg_env_with_handler_cfg, block_env } = self
            .config
            .evm_config
            .next_cfg_and_block_env(&self.parent_header, env_attributes)
            .expect("Valid block environment configuration");
        self.block_env = block_env.clone();
        let env_with_handler_cfg =
            EnvWithHandlerCfg::new_with_cfg_env(cfg_env_with_handler_cfg, block_env, Default::default());

        // send new block params to simulators
        senders
            .send(EvmBlockParams { spec_id: env_with_handler_cfg.spec_id(), env: env_with_handler_cfg.env.clone() })
            .expect("should never fail");

        let evm_config = self.config.evm_config.clone();
        let chain_spec = self.config.evm_config.chain_spec().clone();
        let regolith_active = self
            .config
            .evm_config
            .chain_spec()
            .fork(OpHardfork::Regolith)
            .active_at_timestamp(u64::try_from(env_with_handler_cfg.block.timestamp).unwrap());

        // Configure new EVM to apply pre-execution and must include txs.
        let mut evm = evm_config.evm_with_env(&mut self.frags.db, env_with_handler_cfg);

        // Apply pre-execution changes.
        let block_number = u64::try_from(evm.block().number).unwrap();
        let block_timestamp = u64::try_from(evm.block().timestamp).unwrap();
        evm.db_mut().db.write().set_state_clear_flag(chain_spec.is_spurious_dragon_active_at_block(block_number));

        self.system_caller.apply_beacon_root_contract_call(
            block_timestamp,
            block_number,
            self.payload_attributes.payload_attributes.parent_beacon_block_root,
            &mut evm,
        )?;
        ensure_create2_deployer(chain_spec, block_timestamp, &mut evm.db_mut().db.write())
            .map_err(|_| OpBlockExecutionError::ForceCreate2DeployerFail)?;

        let forced_inclusion_txs = self.payload_attributes.transactions.as_ref().unwrap();

        let mut tx_results = Vec::with_capacity(forced_inclusion_txs.len());

        // Apply must include txs.
        for tx in forced_inclusion_txs.iter() {
            let tx = Arc::new(Transaction::decode(tx.clone()).unwrap());

            // Execute transaction.
            let simulated_tx = simulate_tx_inner(tx, &mut evm, regolith_active, true, true).unwrap();

            self.system_caller.on_state(&simulated_tx.result_and_state.state);
            evm.db_mut().commit(simulated_tx.result_and_state.state.clone());
            tx_results.push(simulated_tx);
        }
        Ok(tx_results)
    }
}

impl<Db> AsRef<BlockEnv> for SequencerContext<Db> {
    fn as_ref(&self) -> &BlockEnv {
        &self.block_env
    }
}
