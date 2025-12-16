use std::{collections::VecDeque, fmt::Display, sync::Arc, time::Instant};

use alloy_consensus::{BlockHeader, EMPTY_OMMER_ROOT_HASH, Header};
use alloy_eips::merge::BEACON_NONCE;
use alloy_rpc_types::engine::{
    BlobsBundleV1, ExecutionPayloadV1, ExecutionPayloadV2, ExecutionPayloadV3, ForkchoiceState,
};
use bop_common::{
    communication::{Producer, SendersSpine, TrackedSenders},
    custom_v4::OpExecutionPayloadEnvelopeV4Patch,
    debug_panic,
    metrics::{Gauge, Metric, MetricsUpdate, metrics_queue},
    p2p::{FragV0, SealV0, StateUpdate},
    shared::SharedState,
    telemetry::{TelemetryUpdate, telemetry_queue},
    time::Timer,
    transaction::{SimulatedTx, Transaction},
    typedefs::*,
};
use bop_db::{DatabaseRead, DatabaseWrite};
use bop_pool::transaction::pool::TxPool;
use op_alloy_rpc_types_engine::OpPayloadAttributes;
use op_revm::OpSpecId;
use reth_chainspec::EthereumHardforks;
use reth_evm::{ConfigureEvm, block::SystemCaller, env::EvmEnv, execute::ProviderError};
use reth_optimism_chainspec::OpChainSpec;
use reth_optimism_consensus::isthmus::withdrawals_root;
use reth_optimism_evm::OpNextBlockEnvAttributes;
use reth_optimism_forks::{OpHardfork, OpHardforks};
use reth_provider::StorageRootProvider;
use revm_primitives::{B256, Bytes, U256, b256};
use tracing::{debug, error, info};
use uuid::Uuid;

use crate::{FragSequence, SequencerConfig, block_sync::BlockSync, sorting::SortingData};

/// These are used to time different parts of the sequencer loop
pub struct SequencerTimers {
    pub start_sequencing: Timer,
    pub block_start: Timer,
    pub send_next: Timer,
    pub apply_tx: Timer,
    pub waiting_for_sims: Timer,
    pub handle_sim: Timer,
    pub seal_frag: Timer,
    pub seal_block: Timer,
    pub handle_deposits: Timer,
}
impl Default for SequencerTimers {
    fn default() -> Self {
        Self {
            start_sequencing: Timer::new("Sequencer-start_sequencing"),
            block_start: Timer::new("Sequencer-block_start"),
            send_next: Timer::new("Sequencer-send_next"),
            apply_tx: Timer::new("Sequencer-apply_tx"),
            waiting_for_sims: Timer::new("Sequencer-wait_for_sims"),
            handle_sim: Timer::new("Sequencer-handle_sim"),
            seal_frag: Timer::new("Sequencer-seal_frag"),
            seal_block: Timer::new("Sequencer-seal_block"),
            handle_deposits: Timer::new("Sequencer-handle_deposits"),
        }
    }
}

pub struct SequencerContext<Db> {
    pub config: SequencerConfig,
    pub db: Db,
    pub shared_state: SharedState<Db>,
    pub tx_pool: TxPool,
    pub deposits: VecDeque<Arc<Transaction>>,
    pub block_env: BlockEnv,
    pub base_fee: u64,
    pub block_executor: BlockSync,
    pub parent_hash: B256,
    pub parent_header: Header,
    pub fork_choice_state: ForkchoiceState,
    pub payload_attributes: Box<OpPayloadAttributes>,
    pub system_caller: SystemCaller<Arc<OpChainSpec>>,
    pub timers: SequencerTimers,
    pub telemetry: Producer<TelemetryUpdate>,
    pub metrics: Producer<MetricsUpdate>,
    pub last_seal_time: Instant,
    pub da_footprint_gas_scalar: Option<u64>,
}

impl<Db: DatabaseRead> SequencerContext<Db> {
    pub fn new(db: Db, shared_state: SharedState<Db>, config: SequencerConfig) -> Self {
        let metrics = metrics_queue().into();

        let block_executor = BlockSync::new(config.evm_config.chain_spec().clone(), metrics);
        let system_caller = SystemCaller::new(config.evm_config.chain_spec().clone());
        Self {
            db,
            metrics,
            shared_state,
            block_executor,
            config,
            system_caller,
            tx_pool: Default::default(),
            deposits: Default::default(),
            fork_choice_state: Default::default(),
            payload_attributes: Default::default(),
            parent_hash: Default::default(),
            parent_header: Default::default(),
            block_env: Default::default(),
            base_fee: Default::default(),
            timers: Default::default(),
            telemetry: telemetry_queue().into(),
            last_seal_time: Instant::now(),
            da_footprint_gas_scalar: None,
        }
    }
}
impl<Db> SequencerContext<Db> {
    pub fn chain_spec(&self) -> &Arc<OpChainSpec> {
        self.config.evm_config.chain_spec()
    }

    pub fn extra_data(&self) -> Bytes {
        let timestamp = self.timestamp_payload_attributes();
        if self.chain_spec().is_jovian_active_at_timestamp(timestamp) {
            // jovian extra data (https://specs.optimism.io/protocol/jovian/exec-engine.html#minimum-base-fee)
            self.payload_attributes
                .get_jovian_extra_data(self.chain_spec().base_fee_params_at_timestamp(timestamp))
                .expect("couldn't get extra data for jovian")
        } else if self.chain_spec().is_holocene_active_at_timestamp(timestamp) {
            self.payload_attributes
                .get_holocene_extra_data(self.chain_spec().base_fee_params_at_timestamp(timestamp))
                .expect("couldn't get extra data for holocene")
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

    pub fn parent_hash(&self) -> B256 {
        self.parent_hash
    }

    pub fn gas_limit(&self) -> u64 {
        self.payload_attributes.gas_limit.expect("should always be set")
    }

    pub fn block_number(&self) -> u64 {
        self.block_env.number.try_into().expect("block number overflow")
    }

    pub fn base_fee(&self) -> u64 {
        self.block_env.basefee.max(self.payload_attributes.min_base_fee.unwrap_or(0))
    }

    pub fn timestamp(&self) -> u64 {
        self.block_env.timestamp.try_into().expect("timestamp overflow")
    }

    pub fn timestamp_payload_attributes(&self) -> u64 {
        self.payload_attributes.payload_attributes.timestamp
    }

    pub fn is_prague_active(&self, timestamp: u64) -> bool {
        self.chain_spec().is_prague_active_at_timestamp(timestamp)
    }
}

// constants for DA footprint related calculations. (see https://github.com/ethereum-optimism/op-geth/blob/d092a020749f4d788501f05ed1be30320ab102b9/core/types/rollup_cost.go#L46-L65)
const ISTHMUS_ATTRIBUTES_LEN: usize = 176;
const JOVIAN_ATTRIBUTES_LEN: usize = 178;
const JOVIAN_L1_ATTRIBUTES_SELECTOR: [u8; 4] = [0x3d, 0xb6, 0xbe, 0x2b];

fn extract_da_footprint_gas_scalar(data: &Bytes) -> Option<u64> {
    if data.len() < JOVIAN_ATTRIBUTES_LEN {
        error!("L1 attributes transaction data too short for DA footprint gas scalar: {}", data.len());
        return None;
    }

    // Future forks need to be added here

    if !data[0..4].eq(&JOVIAN_L1_ATTRIBUTES_SELECTOR) {
        error!("L1 attributes transaction data does not have Jovian selector");
        return None;
    }

    // Extract the last 2 bytes as big-endian u16
    let da_footprint_gas_scalar =
        u16::from_be_bytes([data[JOVIAN_ATTRIBUTES_LEN - 2], data[JOVIAN_ATTRIBUTES_LEN - 1]]);

    Some(da_footprint_gas_scalar as u64)
}

impl<Db: Database> SequencerContext<Db> {
    // This function is only called when the Jovian hardfork is active.
    // see https://github.com/ethereum-optimism/op-geth/blob/d092a020749f4d788501f05ed1be30320ab102b9/core/types/rollup_cost.go#L564-566
    pub fn blob_gas_used_for_jovian(&mut self, txs: &[SimulatedTx]) -> Option<u64> {
        if txs.is_empty() || !txs[0].tx.is_deposit() {
            error!("missing deposit transaction");
            return Some(0);
        }

        let data = &txs[0].tx.as_deposit().expect("should always be deposit").inner().input;
        if data.len() == ISTHMUS_ATTRIBUTES_LEN {
            if !txs.last().expect("txs not empty").tx.is_deposit() {
                error!("unexpected non-deposit transactions in Jovian activation block");
                return Some(0);
            }
            return Some(0);
        }

        match extract_da_footprint_gas_scalar(data) {
            Some(scalar) => {
                self.da_footprint_gas_scalar = Some(scalar);
                let mut da_footprint_gas_used: u64 = 0;
                for tx in txs {
                    if tx.tx.is_deposit() {
                        continue;
                    }
                    da_footprint_gas_used += tx.tx.estimated_tx_compressed_size() * scalar;
                }
                Some(da_footprint_gas_used)
            }
            None => {
                error!("failed to extract DA footprint gas scalar");
                Some(0)
            }
        }
    }

    // https://specs.optimism.io/protocol/jovian/exec-engine.html#da-footprint-block-limit
    pub fn blob_gas_used(&mut self, txs: &[SimulatedTx]) -> Option<u64> {
        let timestamp = self.timestamp_payload_attributes();
        if self.chain_spec().is_jovian_active_at_timestamp(timestamp) {
            self.blob_gas_used_for_jovian(txs)
        } else {
            Some(0)
        }
    }
}

impl<Db: DatabaseRef + Clone + DatabaseRead + Database> SequencerContext<Db> {
    pub fn seal_frag(
        &mut self,
        mut sorting_data: SortingData<Db>,
        frag_seq: &mut FragSequence,
    ) -> (FragV0, Option<StateUpdate>, SortingData<Db>) {
        sorting_data.maybe_apply(self.base_fee);
        sorting_data.send_finished_telemetry();
        info!(
            frag_id = frag_seq.next_seq,
            txs = sorting_data.txs.len(),
            frag_time =% sorting_data.start_t.elapsed(),
            "sealing frag",
        );
        self.shared_state.as_mut().commit_txs(sorting_data.txs.iter_mut());
        self.tx_pool.remove_mined_txs(sorting_data.txs.iter().map(|t| (t.sender_ref(), t)), &mut self.telemetry);
        let (frag, maybe_state) = frag_seq.apply_sorted_frag(sorting_data, self);
        (frag, maybe_state, SortingData::new(frag_seq, self))
    }
}

impl<Db: DatabaseRead + Database<Error: Into<ProviderError> + Display> + StorageRootProvider> SequencerContext<Db> {
    pub fn handle_tx(&mut self, tx: Arc<Transaction>, senders: &SendersSpine<Db>) {
        if tx.is_deposit() {
            self.deposits.push_back(tx);
            return;
        }
        if self.tx_pool.handle_tx(
            tx.clone(),
            self.shared_state.as_ref(),
            self.base_fee(),
            false,
            self.config.simulate_tof_in_pools.then_some(senders),
        ) {
            TelemetryUpdate::send(tx.uuid, tx.to_added_to_pool_telemetry(), &mut self.telemetry);
        }
    }

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
        let simulator_evm_block_params = self.new_block_params();
        self.block_env = simulator_evm_block_params.block_env.clone();
        self.base_fee = self.base_fee();

        // send new block params to simulators
        senders.send(simulator_evm_block_params.clone()).expect("should never fail");
        let n_force_include_txs =
            self.payload_attributes.transactions.as_ref().map(|txs| txs.len()).unwrap_or_default();
        let max_block_da = self.config.da_config.max_da_block_size();
        let seq = FragSequence::new(
            self.gas_limit(),
            max_block_da,
            self.block_number(),
            self.timestamp(),
            self.base_fee(),
            n_force_include_txs,
        );

        debug!("new frag sequence");
        let mut sorting = SortingData::new(&seq, self);

        // Update block height metric
        MetricsUpdate::send(
            Uuid::new_v4(),
            Metric::SetGauge(Gauge::GatewayBlockHeight, self.block_number() as f64),
            &mut self.metrics,
        );

        // Apply must include
        sorting.apply_block_start_to_state(self, simulator_evm_block_params).expect("shouldn't fail");
        self.tx_pool.remove_mined_txs(sorting.txs.iter().map(|t| (t.sender_ref(), t)), &mut self.telemetry);

        (seq, sorting)
    }

    pub fn new_block_params(&mut self) -> EvmEnv<OpSpecId> {
        let attributes = &self.payload_attributes;
        let env_attributes = OpNextBlockEnvAttributes {
            timestamp: attributes.payload_attributes.timestamp,
            suggested_fee_recipient: attributes.payload_attributes.suggested_fee_recipient,
            prev_randao: attributes.payload_attributes.prev_randao,
            gas_limit: attributes.gas_limit.unwrap(),
            parent_beacon_block_root: self.payload_attributes.payload_attributes.parent_beacon_block_root,
            extra_data: self.payload_attributes.eip_1559_params.unwrap_or_default().into(),
        };

        self.config
            .evm_config
            .next_evm_env(&self.parent_header, &env_attributes)
            .expect("couldn't construct the next evm env")
    }

    pub fn seal_last_frag(
        &mut self,
        frag_seq: &mut FragSequence,
        last_frag: SortingData<Db>,
    ) -> (FragV0, Option<StateUpdate>, Option<u64>) {
        let (mut frag_msg, maybe_update, _) = self.seal_frag(last_frag, frag_seq);
        frag_msg.is_last = true;
        let blob_gas_used = self.blob_gas_used(&frag_seq.txs);
        frag_msg.blob_gas_used = blob_gas_used.unwrap_or_default();
        (frag_msg, maybe_update, blob_gas_used)
    }

    /// Finalize the block after the last frag has been sealed
    pub fn seal_block(
        &mut self,
        frag_seq: FragSequence,
        blob_gas_used: Option<u64>,
    ) -> (SealV0, OpExecutionPayloadEnvelopeV4Patch) {
        frag_seq.sorting_telemetry.report(&self.metrics);
        let gas_used = frag_seq.gas_used;
        let canyon_active = self.chain_spec().fork(OpHardfork::Canyon).active_at_timestamp(self.timestamp());
        let (transactions, transactions_root, receipts_root, logs_bloom) =
            frag_seq.encoded_txs_roots_bloom(canyon_active);

        let state_changes = self.shared_state.as_mut().take_state_changes();
        let state_root = self.db.calculate_state_root(&state_changes).unwrap().0;

        let (withdrawals_root, requests_hash) = if self.is_prague_active(self.timestamp()) {
            (
                Some(
                    withdrawals_root(&state_changes, &self.db)
                        .expect("something wrong with withdrawals root calculation"),
                ),
                Some(b256!("0xe3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")),
            )
        } else {
            (Some(b256!("0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421")), None)
        };

        let extra_data = self.extra_data();

        let parent_beacon_block_root = self.parent_beacon_block_root();

        let header = Header {
            parent_hash: self.parent_hash,
            ommers_hash: EMPTY_OMMER_ROOT_HASH,
            beneficiary: self.block_env.beneficiary,
            state_root,
            transactions_root,
            receipts_root,
            withdrawals_root,
            logs_bloom,
            timestamp: self.block_env.timestamp.try_into().expect("timestamp overflow"),
            mix_hash: self.block_env.prevrandao.unwrap_or_default(),
            nonce: BEACON_NONCE.into(),
            base_fee_per_gas: Some(self.base_fee()),
            number: self.block_env.number.try_into().expect("block number overflow"),
            gas_limit: self.block_env.gas_limit,
            difficulty: U256::ZERO,
            gas_used,
            extra_data: extra_data.clone(),
            parent_beacon_block_root,
            blob_gas_used,
            excess_blob_gas: Some(0),
            requests_hash,
        };

        let v1 = ExecutionPayloadV1 {
            parent_hash: self.parent_hash,
            fee_recipient: self.block_env.beneficiary,
            state_root,
            receipts_root,
            logs_bloom,
            prev_randao: self.block_env.prevrandao.unwrap_or_default(),
            block_number: self.block_env.number.try_into().expect("block number overflow"),
            gas_limit: self.block_env.gas_limit,
            gas_used,
            timestamp: self.block_env.timestamp.try_into().expect("timestamp overflow"),
            extra_data,
            base_fee_per_gas: U256::from(self.base_fee()),
            block_hash: header.hash_slow(),
            transactions,
        };
        let seal = SealV0 {
            total_frags: frag_seq.next_seq,
            block_number: self.block_env.number.try_into().expect("block number overflow"),
            gas_used,
            gas_limit: self.block_env.gas_limit,
            parent_hash: self.parent_hash,
            transactions_root,
            receipts_root,
            state_root,
            block_hash: v1.block_hash,
            blob_gas_used: blob_gas_used.unwrap_or_default(),
        };

        let block_duration = frag_seq.start_t.elapsed().as_secs();
        let mgas = (gas_used / 10_000) as f64 / 100.0;
        let txs = frag_seq.txs.len() as f64;
        let tps = txs / block_duration;
        let mgas_s = mgas / block_duration;
        let is_jovian_active = self.chain_spec().is_jovian_active_at_timestamp(self.timestamp_payload_attributes());
        let is_prague_active = self.is_prague_active(self.timestamp());
        info!(
            "sealed block {} with {} txs, {mgas} MGas ({:.2} MGas/s) jovian_active: {} prague_active: {}",
            seal.block_number, txs, mgas_s, is_jovian_active, is_prague_active
        );

        let uuid = Uuid::new_v4();
        MetricsUpdate::send(
            uuid,
            Metric::SetGauge(Gauge::GatewayBlockBuildDurationSecs, block_duration),
            &mut self.metrics,
        );
        MetricsUpdate::send(uuid, Metric::SetGauge(Gauge::GatewayBlockTxCount, txs), &mut self.metrics);
        MetricsUpdate::send(uuid, Metric::SetGauge(Gauge::GatewayBlockGasUsed, gas_used as f64), &mut self.metrics);
        MetricsUpdate::send(uuid, Metric::SetGauge(Gauge::GatewayTps, tps), &mut self.metrics);
        MetricsUpdate::send(uuid, Metric::SetGauge(Gauge::GatewayMGasS, mgas_s), &mut self.metrics);

        let payload_inner = ExecutionPayloadV3 {
            payload_inner: ExecutionPayloadV2 { payload_inner: v1, withdrawals: vec![] },
            blob_gas_used: blob_gas_used.unwrap_or(0),
            excess_blob_gas: 0,
        };
        (seal, OpExecutionPayloadEnvelopeV4Patch {
            execution_payload: op_alloy_rpc_types_engine::OpExecutionPayloadV4 {
                payload_inner,
                withdrawals_root: withdrawals_root.unwrap_or(B256::ZERO),
            },
            block_value: frag_seq.payment.to(),
            blobs_bundle: BlobsBundleV1::new(vec![]),
            should_override_builder: false,
            parent_beacon_block_root: parent_beacon_block_root.expect("should always be set"),
            execution_requests: vec![],
        })
    }
}
impl<Db: DatabaseWrite + DatabaseRead> SequencerContext<Db> {
    /// Commit new to DB, either due to syncing or due to New Payload EngineApi message.
    /// If it was based on a new payload message rather than blocksync, we pass the base_fee,
    /// and clear the existing pool based on that
    /// Returns a list of block numbers to fetch. This will be used in the case of a reorg.
    pub fn commit_block(&mut self, block: &BlockSyncMessage) -> Option<(u64, u64)> {
        let blocks_to_fetch = match self.block_executor.commit_block(block, &self.db, true, &mut self.telemetry) {
            Ok(blocks_to_fetch) => blocks_to_fetch,
            Err(e) => {
                debug_panic!("couldn't commit block: {e}");
                let bn = block.number();
                return Some((bn, bn));
            }
        };

        self.parent_header = block.header().clone();
        self.parent_hash = block.hash_slow();

        // Update block height metric when syncing
        MetricsUpdate::send(
            Uuid::new_v4(),
            Metric::SetGauge(Gauge::GatewayBlockHeight, block.number() as f64),
            &mut self.metrics,
        );

        // Completely wipe active txs as they may contain valid nonces with out of date sim results.
        self.tx_pool.pending_orders.clear();
        self.tx_pool.remove_mined_txs(block.transactions_with_sender(), &mut self.telemetry);

        if let Some(base_fee) = block.base_fee_per_gas {
            self.base_fee = base_fee;
            self.tx_pool.handle_new_frag(base_fee, self.shared_state.as_ref(), false, None);
        }

        blocks_to_fetch
    }
}

impl<Db> AsRef<BlockEnv> for SequencerContext<Db> {
    fn as_ref(&self) -> &BlockEnv {
        &self.block_env
    }
}
