use std::{fmt::Display, sync::Arc};

use alloy_consensus::{Header, EMPTY_OMMER_ROOT_HASH};
use alloy_eips::merge::BEACON_NONCE;
use alloy_rpc_types::engine::{
    BlobsBundleV1, ExecutionPayloadV1, ExecutionPayloadV2, ExecutionPayloadV3, ForkchoiceState,
};
use bop_common::{
    communication::{
        messages::{BlockSyncMessage, EvmBlockParams},
        SendersSpine, TrackedSenders,
    },
    db::DBFrag,
    p2p::{FragV0, SealV0},
    time::Timer,
    transaction::SimulatedTx,
};
use bop_db::{DatabaseRead, DatabaseWrite};
use bop_pool::transaction::pool::TxPool;
use op_alloy_rpc_types_engine::{OpExecutionPayloadEnvelopeV3, OpPayloadAttributes};
use reth_evm::{
    env::EvmEnv, execute::ProviderError, system_calls::SystemCaller, ConfigureEvmEnv, NextBlockEnvAttributes,
};
use reth_optimism_chainspec::OpChainSpec;
use reth_optimism_evm::OpEvmConfig;
use reth_optimism_forks::{OpHardfork, OpHardforks};
use revm::{Database, DatabaseRef};
use revm_primitives::{b256, BlockEnv, Bytes, EnvWithHandlerCfg, B256, U256};

use crate::{block_sync::BlockSync, sorting::SortingData, FragSequence, SequencerConfig};

/// These are used to time different parts of the sequencer loop
pub struct SequencerTimers {
    pub start_sequencing: Timer,
    pub block_start: Timer,
    pub apply_and_send_next: Timer,
    pub apply_tx: Timer,
    pub waiting_for_sims: Timer,
    pub handle_sim: Timer,
    pub seal_frag: Timer,
    pub seal_block: Timer,
}
impl Default for SequencerTimers {
    fn default() -> Self {
        Self {
            start_sequencing: Timer::new("Sequencer-start_sequencing"),
            block_start: Timer::new("Sequencer-block_start"),
            apply_and_send_next: Timer::new("Sequencer-apply_and_send"),
            apply_tx: Timer::new("Sequencer-apply_tx"),
            waiting_for_sims: Timer::new("Sequencer-wait_for_sims"),
            handle_sim: Timer::new("Sequencer-handle_sim"),
            seal_frag: Timer::new("Sequencer-seal_frag"),
            seal_block: Timer::new("Sequencer-seal_block"),
        }
    }
}

pub struct SequencerContext<Db> {
    pub config: SequencerConfig,
    pub db: Db,
    /// This is a wrapper around db to tag frags onto before
    /// sealing the block and commmiting it to db.
    /// Each time a frag is done being sorted it gets applied and
    /// a new DBSorting gets created around db_frag to which individual
    /// txs will be attached
    /// Furthermore, this db is shared with the RPC serving layer, hence it
    /// should not be outright overwritten. Only the internal db should be
    /// changed when a sorted frag is applied or (reset) when a block is sealed and committed to
    /// the persisted underlying db.
    pub db_frag: DBFrag<Db>,
    pub tx_pool: TxPool,
    pub block_env: BlockEnv,
    pub base_fee: u64,
    pub block_executor: BlockSync,
    pub parent_hash: B256,
    pub parent_header: Header,
    pub fork_choice_state: ForkchoiceState,
    pub payload_attributes: Box<OpPayloadAttributes>,
    pub system_caller: SystemCaller<OpEvmConfig, OpChainSpec>,
    pub timers: SequencerTimers,
}

impl<Db: DatabaseRead> SequencerContext<Db> {
    pub fn new(db: Db, db_frag: DBFrag<Db>, config: SequencerConfig) -> Self {
        let block_executor = BlockSync::new(config.evm_config.chain_spec().clone());
        let system_caller = SystemCaller::new(config.evm_config.clone(), config.evm_config.chain_spec().clone());
        Self {
            db,
            db_frag,
            block_executor,
            config,
            system_caller,
            tx_pool: Default::default(),
            fork_choice_state: Default::default(),
            payload_attributes: Default::default(),
            parent_hash: Default::default(),
            parent_header: Default::default(),
            block_env: Default::default(),
            base_fee: Default::default(),
            timers: Default::default(),
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

    pub fn regolith_active(&self, timestamp: u64) -> bool {
        self.config.evm_config.chain_spec().fork(OpHardfork::Regolith).active_at_timestamp(timestamp)
    }

    pub fn parent_beacon_block_root(&self) -> Option<B256> {
        self.payload_attributes.payload_attributes.parent_beacon_block_root
    }

    pub fn gas_limit(&self) -> u64 {
        self.payload_attributes.gas_limit.expect("should always be set")
    }

    pub fn block_number(&self) -> u64 {
        self.block_env.number.to()
    }

    pub fn base_fee(&self) -> u64 {
        self.block_env.basefee.to()
    }

    pub fn timestamp(&self) -> u64 {
        self.block_env.timestamp.to()
    }
}
impl<Db: DatabaseRef + Clone> SequencerContext<Db> {
    pub fn seal_frag(
        &mut self,
        mut sorting_data: SortingData<Db>,
        frag_seq: &mut FragSequence,
    ) -> (FragV0, SortingData<Db>) {
        for t in &mut sorting_data.txs {
            let state = t.take_state();
            if state.is_empty() {
                continue;
            }
            self.system_caller.on_state(&state);
            self.db_frag.commit_flat_changes(state);
        }
        self.tx_pool.remove_mined_txs(sorting_data.txs.iter());
        (frag_seq.apply_sorted_frag(sorting_data), SortingData::new(frag_seq, self))
    }
}

impl<Db: DatabaseRead + Database<Error: Into<ProviderError> + Display>> SequencerContext<Db> {
    /// Processes a new block from the sequencer by:
    /// 1. Updating EVM environments
    /// 2. Applying pre-execution changes
    /// 3. Processing forced inclusion transactions
    pub fn start_sequencing(
        &mut self,
        attributes: Box<OpPayloadAttributes>,
        senders: &SendersSpine<Db>,
    ) -> (FragSequence, SortingData<Db>) {
        self.payload_attributes = attributes;
        let (simulator_evm_block_params, env_with_handler_cfg) = self.new_block_params();
        self.block_env = simulator_evm_block_params.env.block.clone();
        self.base_fee = self.block_env.basefee.to();

        // send new block params to simulators
        senders.send(simulator_evm_block_params).expect("should never fail");

        let seq = FragSequence::new(self.gas_limit(), self.block_number());
        let mut sorting = SortingData::new(&seq, self);

        sorting.apply_block_start_to_state(self, env_with_handler_cfg).expect("shouldn't fail");
        self.tx_pool.remove_mined_txs(sorting.txs.iter());
        (seq, sorting)
    }

    fn new_block_params(&mut self, attributes: &Box<OpPayloadAttributes>) -> (EvmBlockParams, EnvWithHandlerCfg) {
    fn new_block_params(&mut self) -> (EvmBlockParams, EnvWithHandlerCfg) {
        let attributes = &self.payload_attributes;
        let env_attributes = NextBlockEnvAttributes {
            timestamp: attributes.payload_attributes.timestamp,
            suggested_fee_recipient: attributes.payload_attributes.suggested_fee_recipient,
            prev_randao: attributes.payload_attributes.prev_randao,
            gas_limit: attributes.gas_limit.unwrap(),
        };

        let EvmEnv { cfg_env_with_handler_cfg, block_env } = self
            .config
            .evm_config
            .next_cfg_and_block_env(&self.parent_header, env_attributes)
            .expect("Valid block environment configuration");

        let env_with_handler_cfg =
            EnvWithHandlerCfg::new_with_cfg_env(cfg_env_with_handler_cfg, block_env, Default::default());
        let simulator_evm_block_params =
            EvmBlockParams { spec_id: env_with_handler_cfg.spec_id(), env: env_with_handler_cfg.env.clone() };
        (simulator_evm_block_params, env_with_handler_cfg)
    }

    pub fn seal_block(
        &mut self,
        mut frag_seq: FragSequence,
        last_frag: SortingData<Db>,
    ) -> (FragV0, SealV0, OpExecutionPayloadEnvelopeV3) {
        let (mut frag_msg, _) = self.seal_frag(last_frag, &mut frag_seq);
        frag_msg.is_last = true;
        let gas_used = frag_seq.gas_used;

        let state_changes = self.db_frag.take_state_changes();
        let state_root = self.db.calculate_state_root(&state_changes).unwrap().0;

        let canyon_active =
            self.chain_spec().fork(OpHardfork::Canyon).active_at_timestamp(u64::try_from(self.timestamp()).unwrap());

        let (transactions, transactions_root, receipts_root, logs_bloom) =
            frag_seq.encoded_txs_roots_bloom(canyon_active);

        let extra_data = self.extra_data();

        let parent_beacon_block_root = self.parent_beacon_block_root();

        let header = Header {
            parent_hash: self.parent_hash,
            ommers_hash: EMPTY_OMMER_ROOT_HASH,
            beneficiary: self.block_env.coinbase,
            state_root,
            transactions_root,
            receipts_root,
            withdrawals_root: Some(b256!("0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421")),
            logs_bloom,
            timestamp: self.block_env.timestamp.to(),
            mix_hash: self.block_env.prevrandao.unwrap_or_default(),
            nonce: BEACON_NONCE.into(),
            base_fee_per_gas: Some(self.block_env.basefee.to()),
            number: self.block_env.number.to(),
            gas_limit: self.block_env.gas_limit.to(),
            difficulty: U256::ZERO,
            gas_used,
            extra_data: extra_data.clone(),
            parent_beacon_block_root,
            blob_gas_used: Some(0),
            excess_blob_gas: Some(0),
            requests_hash: None,
        };

        let v1 = ExecutionPayloadV1 {
            parent_hash: self.parent_hash,
            fee_recipient: self.block_env.coinbase,
            state_root,
            receipts_root,
            logs_bloom,
            prev_randao: self.block_env.prevrandao.unwrap_or_default(),
            block_number: self.block_env.number.to(),
            gas_limit: self.block_env.gas_limit.to(),
            gas_used,
            timestamp: self.block_env.timestamp.to(),
            extra_data,
            base_fee_per_gas: self.block_env.basefee,
            block_hash: header.hash_slow(),
            transactions,
        };
        let seal = SealV0 {
            total_frags: frag_seq.next_seq,
            block_number: self.block_env.number.to(),
            gas_used,
            gas_limit: self.block_env.gas_limit.to(),
            parent_hash: self.parent_hash,
            transactions_root,
            receipts_root,
            state_root,
            block_hash: v1.block_hash,
        };
        (frag_msg, seal, OpExecutionPayloadEnvelopeV3 {
            execution_payload: ExecutionPayloadV3 {
                payload_inner: ExecutionPayloadV2 { payload_inner: v1, withdrawals: vec![] },
                blob_gas_used: 0,
                excess_blob_gas: 0,
            },
            block_value: frag_seq.payment.to(),
            blobs_bundle: BlobsBundleV1::new(vec![]),
            should_override_builder: false,
            parent_beacon_block_root: parent_beacon_block_root.expect("should always be set"),
        })
    }
}
impl<Db: DatabaseWrite + DatabaseRead> SequencerContext<Db> {
    /// Commit new to DB, either due to syncing or due to New Payload EngineApi message.
    /// If it was based on a new payload message rather than blocksync, we pass Some(base_fee),
    /// and clear the xstx pool based on that
    pub fn commit_block(&mut self, block: &BlockSyncMessage, base_fee: Option<u64>) {
        self.block_executor.commit_block(block, &self.db, true).expect("couldn't commit block");
        self.db_frag.reset();

        self.parent_header = block.header.clone();
        self.parent_hash = block.hash_slow();

        if let Some(base_fee) = base_fee {
            self.base_fee = base_fee;
        }

        if let Some(base_fee) = base_fee {
            self.tx_pool.handle_new_block(block.body.transactions.iter(), base_fee, &self.db_frag, false, None);
        }
    }
}

impl<Db> AsRef<BlockEnv> for SequencerContext<Db> {
    fn as_ref(&self) -> &BlockEnv {
        &self.block_env
    }
}
