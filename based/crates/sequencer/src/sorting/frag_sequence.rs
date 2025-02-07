use std::{fmt::Display, sync::Arc};

use alloy_consensus::{proofs::ordered_trie_root_with_encoder, Header, EMPTY_OMMER_ROOT_HASH};
use alloy_eips::{eip2718::Encodable2718, merge::BEACON_NONCE};
use alloy_primitives::{Bloom, U256};
use alloy_rpc_types::engine::{BlobsBundleV1, ExecutionPayloadV1, ExecutionPayloadV2, ExecutionPayloadV3};
use bop_common::{
    db::{flatten_state_changes, DBFrag, DBSorting},
    p2p::{FragV0, SealV0},
    transaction::SimulatedTx,
};
use bop_db::DatabaseRead;
use op_alloy_rpc_types_engine::OpExecutionPayloadEnvelopeV3;
use reth_evm::execute::ProviderError;
use reth_optimism_chainspec::OpChainSpec;
use reth_optimism_forks::OpHardfork;
use revm::{
    db::{states::bundle_state::BundleRetention, BundleState, State},
    Database, DatabaseCommit, DatabaseRef,
};
use revm_primitives::{hex, BlockEnv, Bytes, EvmState, B256};

use crate::sorting::InSortFrag;

/// Sequence of frags applied on the last block
#[derive(Clone, Debug)]
pub struct FragSequence<Db> {
    pub db: DBFrag<Db>,
    gas_remaining: u64,
    payment: U256,
    txs: Vec<SimulatedTx>,
    /// Next frag index
    next_seq: u64,
    /// Block number for all frags in this block
    block_number: u64,
}
impl<Db> FragSequence<Db> {
    pub fn set_gas_limit(&mut self, gas_limit: u64) {
        self.gas_remaining = gas_limit;
    }

    pub fn db_ref(&self) -> &DBFrag<Db> {
        &self.db
    }

    // pub fn apply_top_of_block(&mut self, top_of_block: TopOfBlockResult) -> FragV0
    // where
    //     Db: DatabaseRead + Database<Error: Into<ProviderError> + Display>,
    // {
    // todo!();
    // self.gas_remaining -= top_of_block.gas_used();
    // self.payment += top_of_block.payment();
    // self.top_of_block_bundle = top_of_block.state;
    // self.top_of_block_changes = top_of_block.changes;
    // tracing::info!("###################{}",self.db.calculate_state_root(&top_of_block.state).unwrap().0);
    // self.db.commit_flat_changes(top_of_block.flat_state_changes);
    // self.db.commit( );
    // let msg = FragV0::new(
    //     self.block_number,
    //     self.next_seq,
    //     top_of_block.forced_inclusion_txs.iter().map(|tx| tx.tx.as_ref()),
    //     false,
    // );
    // self.txs.extend(top_of_block.forced_inclusion_txs);
    // msg
    // }

    /// When a new block is received, we clear all the temp state on the db
    pub fn reset(&mut self, gas_limit: u64, forced_inclusion_txs: Vec<SimulatedTx>) {
        self.gas_remaining = gas_limit - forced_inclusion_txs.iter().map(|t| t.gas_used()).sum::<u64>();
        self.payment = forced_inclusion_txs.iter().map(|t| t.payment).sum();
        self.txs = forced_inclusion_txs;
        self.next_seq = 0;
        todo!()
        // self.db.reset(db);
    }
}
impl<Db: Clone> FragSequence<Db> {
    /// Builds a new in-sort frag
    pub fn create_in_sort(&self) -> InSortFrag<Db> {
        let db_sort = DBSorting::new(self.db());
        InSortFrag::new(db_sort, self.gas_remaining)
    }

    // TODO: remove this and move to sortign data
    pub fn db(&self) -> DBFrag<Db> {
        self.db.clone()
    }
}

impl<Db: DatabaseRead> FragSequence<Db> {
    pub fn new(db: DBFrag<Db>, max_gas: u64) -> Self {
        let block_number = db.head_block_number().expect("can't get block number") + 1;
        Self { db, gas_remaining: max_gas, payment: U256::ZERO, txs: vec![], next_seq: 0, block_number }
    }

    pub fn is_valid(&self, state_id: u64) -> bool {
        state_id == self.db.state_id()
    }
}

impl<Db: DatabaseRef> FragSequence<Db> {
    /// Creates a new frag, all subsequent frags will be built on top of this one
    pub fn apply_sorted_frag(&mut self, in_sort: InSortFrag<Db>) -> FragV0 {
        self.gas_remaining -= in_sort.gas_used;
        self.payment += in_sort.payment;

        let msg = FragV0::new(self.block_number, self.next_seq, in_sort.txs.iter().map(|tx| tx.tx.as_ref()), false);

        self.db.commit(in_sort.txs.iter());
        self.txs.extend(in_sort.txs);
        self.next_seq += 1;

        msg
    }
}

impl<Db> FragSequence<Db> {
    pub fn seal_block(
        &mut self,
        block_env: &BlockEnv,
        parent_hash: B256,
        parent_beacon_block_root: B256,
        chain_spec: &Arc<OpChainSpec>,
        extra_data: Bytes,
    ) -> (SealV0, OpExecutionPayloadEnvelopeV3)
    where
        Db: DatabaseRead + Database<Error: Into<ProviderError> + Display>,
    {
        let bundle = self.db.db.write().take_bundle();
        // self.db.commit_flat_changes(state_changes);
        // self.db.merge_transitions(BundleRetention::Reverts);
        // let state_root = self.db.calculate_state_root(&state_changes_to_bundle_state(&self.db,
        // flatten_state_changes(self.top_of_block_changes.clone())).unwrap()).unwrap().0;
        let state_root = self.db.calculate_state_root(&bundle).unwrap().0;

        let mut receipts = Vec::with_capacity(self.txs.len());
        let mut transactions = Vec::with_capacity(self.txs.len());
        let mut logs_bloom = Bloom::ZERO;
        let mut gas_used = 0;

        let canyon_active =
            chain_spec.fork(OpHardfork::Canyon).active_at_timestamp(u64::try_from(block_env.timestamp).unwrap());
        for t in self.txs.iter() {
            gas_used += t.result_and_state.result.gas_used();
            let receipt = t.receipt(gas_used, canyon_active);
            logs_bloom |= receipt.logs_bloom;
            receipts.push(receipt);
            transactions.push(t.tx.encode());
        }

        let receipts_root = ordered_trie_root_with_encoder(&receipts, |r, buf| {
            r.encode_2718(buf);
        });

        let transactions_root = ordered_trie_root_with_encoder(&self.txs, |tx, buf| tx.encode_2718(buf));
        let header = Header {
            parent_hash,
            ommers_hash: EMPTY_OMMER_ROOT_HASH,
            beneficiary: block_env.coinbase,
            state_root,
            transactions_root,
            receipts_root,
            withdrawals_root: Some(hex!("0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421").into()),
            logs_bloom,
            timestamp: block_env.timestamp.to(),
            mix_hash: block_env.prevrandao.unwrap_or_default(),
            nonce: BEACON_NONCE.into(),
            base_fee_per_gas: Some(block_env.basefee.to()),
            number: block_env.number.to(),
            gas_limit: block_env.gas_limit.to(),
            difficulty: U256::ZERO,
            gas_used,
            extra_data: extra_data.clone(),
            parent_beacon_block_root: Some(parent_beacon_block_root),
            blob_gas_used: Some(0),
            excess_blob_gas: Some(0),
            requests_hash: None,
        };

        let v1 = ExecutionPayloadV1 {
            parent_hash,
            fee_recipient: block_env.coinbase,
            state_root,
            receipts_root,
            logs_bloom,
            prev_randao: block_env.prevrandao.unwrap_or_default(),
            block_number: block_env.number.to(),
            gas_limit: block_env.gas_limit.to(),
            gas_used,
            timestamp: block_env.timestamp.to(),
            extra_data,
            base_fee_per_gas: block_env.basefee,
            block_hash: header.hash_slow(),
            transactions,
        };
        let seal = SealV0 {
            total_frags: self.next_seq,
            block_number: block_env.number.to(),
            gas_used,
            gas_limit: block_env.gas_limit.to(),
            parent_hash,
            transactions_root,
            receipts_root,
            state_root,
            block_hash: v1.block_hash,
        };
        tracing::info!("seal: {seal:#?}");
        (seal, OpExecutionPayloadEnvelopeV3 {
            execution_payload: ExecutionPayloadV3 {
                payload_inner: ExecutionPayloadV2 { payload_inner: v1, withdrawals: vec![] },
                blob_gas_used: 0,
                excess_blob_gas: 0,
            },
            block_value: self.payment,
            blobs_bundle: BlobsBundleV1::new(vec![]),
            should_override_builder: false,
            parent_beacon_block_root,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use alloy_consensus::Signed;
    use alloy_primitives::U256;
    use alloy_provider::ProviderBuilder;
    use bop_common::{
        actor::{Actor, ActorConfig},
        communication::{
            messages::{SequencerToSimulator, SimulatorToSequencer, SimulatorToSequencerMsg},
            Spine, TrackedSenders,
        },
        db::DBFrag,
    };
    use bop_db::AlloyDB;
    use bop_simulator::Simulator;
    use op_alloy_consensus::{OpTxEnvelope, OpTypedTransaction};
    use reqwest::{Client, Url};
    use reth_optimism_chainspec::{OpChainSpecBuilder, BASE_SEPOLIA};
    use reth_optimism_evm::OpEvmConfig;
    use reth_primitives_traits::{Block, SignedTransaction};
    use revm_primitives::{BlobExcessGasAndPrice, BlockEnv};

    use crate::{block_sync::fetch_blocks::fetch_block, sorting::FragSequence};

    const ENV_RPC_URL: &str = "BASE_RPC_URL";
    const TEST_BASE_RPC_URL: &str = "https://base-rpc.publicnode.com";

    #[test]
    fn test_block_seal_with_alloydb() {
        let rt = Arc::new(tokio::runtime::Runtime::new().unwrap());

        // Get RPC URL from environment
        let rpc_url = std::env::var(ENV_RPC_URL).unwrap_or(TEST_BASE_RPC_URL.to_string());
        let rpc_url = Url::parse(&rpc_url).unwrap();
        tracing::info!("RPC URL: {}", rpc_url);

        // Create the block executor.
        let chain_spec = Arc::new(OpChainSpecBuilder::base_sepolia().build());

        // Fetch the block from the RPC.
        let provider = ProviderBuilder::new().network().on_http(rpc_url);
        let block = rt.block_on(async { fetch_block(25771900, &provider).await });

        let header = block.block.header();

        let block_env = BlockEnv {
            number: U256::from(header.number),
            coinbase: (*header.beneficiary).into(),
            timestamp: U256::from(header.timestamp),
            difficulty: header.difficulty,
            basefee: U256::from(header.base_fee_per_gas.unwrap()),
            gas_limit: U256::from(header.gas_limit),
            prevrandao: Some(header.mix_hash),
            blob_excess_gas_and_price: header.excess_blob_gas.map(|ebg| BlobExcessGasAndPrice::new(ebg, false)),
        };

        // Create the alloydb.
        let client = ProviderBuilder::new().network().on_http(rpc_url);
        let alloy_db = AlloyDB::new(client, block.block.header.number, rt);
        let evm_config = OpEvmConfig::new(BASE_SEPOLIA.clone());

        // Simulate the txs in the block and add to a frag.
        let db_frag: DBFrag<_> = alloy_db.clone().into();
        let spine = Spine::default();

        let sim_connections = spine.to_connections("sim");
        let sim_db = db_frag.clone();

        // Simulator
        let _sim_handle =
            std::thread::spawn(move || Simulator::create_and_run(sim_connections, sim_db, ActorConfig::default(), 0));
        let mut seq = FragSequence::new(db_frag, 300_000_000);
        let mut sorting_db = seq.create_in_sort();

        let mut connections = spine.to_connections("test");
        connections.send(block_env.clone());

        for signed_tx in &block.block.body.transactions {
            let sender = signed_tx.recover_signer().unwrap();
            let typed_tx: &OpTypedTransaction = &signed_tx.transaction;
            let envelope: OpTxEnvelope = match typed_tx {
                OpTypedTransaction::Legacy(x) => {
                    Signed::new_unchecked(x.clone(), signed_tx.signature().clone(), *signed_tx.tx_hash()).into()
                }
                OpTypedTransaction::Eip2930(x) => {
                    Signed::new_unchecked(x.clone(), signed_tx.signature().clone(), *signed_tx.tx_hash()).into()
                }
                OpTypedTransaction::Eip1559(x) => {
                    Signed::new_unchecked(x.clone(), signed_tx.signature().clone(), *signed_tx.tx_hash()).into()
                }
                OpTypedTransaction::Eip7702(x) => {
                    Signed::new_unchecked(x.clone(), signed_tx.signature().clone(), *signed_tx.tx_hash()).into()
                }
                OpTypedTransaction::Deposit(x) => x.clone().into(),
            };

            let bop_tx = Arc::new(bop_common::transaction::Transaction::new(envelope, sender));
            connections.senders().send(SequencerToSimulator::SimulateTx(bop_tx, sorting_db.state())).unwrap();
            connections.receive(|msg: SimulatorToSequencer<_>, _senders| {
                if let SimulatorToSequencerMsg::Tx(Ok(tx)) = msg.msg {
                    sorting_db.apply_tx(tx);
                }
            });
        }

        seq.apply_sorted_frag(sorting_db);

        let (_seal, payload) = seq.seal_block(&block_env, chain_spec, block.block.header.parent_hash);
        assert_eq!(block.block.header.state_root, payload.execution_payload.payload_inner.payload_inner.state_root);
    }
}
