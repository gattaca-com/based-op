use alloy_consensus::{proofs::ordered_trie_root_with_encoder, Eip658Value, Receipt, ReceiptWithBloom, Transaction};
use alloy_eips::eip2718::Encodable2718;
use alloy_primitives::{Bloom, U256};
use alloy_rpc_types::TransactionReceipt;
use bop_common::{p2p::FragV0, transaction::SimulatedTx};
use op_alloy_consensus::{OpDepositReceipt, OpDepositReceiptWithBloom, OpReceiptEnvelope, OpTxType};
use op_alloy_rpc_types::{L1BlockInfo, OpTransactionReceipt};
use revm_primitives::{Bytes, B256};

use super::{sorting_data::SortingTelemetry, SortingData};
use crate::context::SequencerContext;

/// Sequence of frags applied on the last block
#[derive(Clone, Debug)]
pub struct FragSequence {
    pub gas_remaining: u64,
    pub gas_used: u64,
    pub payment: U256,
    pub txs: Vec<SimulatedTx>,
    /// Next frag index
    pub next_seq: u64,
    /// Block number and timestamp shared by all frags of this sequence
    block_number: u64,
    block_timestamp: u64,

    pub sorting_telemetry: SortingTelemetry,
}
impl FragSequence {
    pub fn new(gas_remaining: u64, block_number: u64, block_timestamp: u64) -> Self {
        Self {
            gas_remaining,
            gas_used: 0,
            payment: U256::ZERO,
            txs: vec![],
            block_number,
            block_timestamp,
            next_seq: 0,
            sorting_telemetry: Default::default(),
        }
    }

    pub fn set_gas_limit(&mut self, gas_limit: u64) {
        self.gas_remaining = gas_limit;
    }

    pub fn apply_sorted_frag<Db>(&mut self, in_sort: SortingData<Db>, ctx: &mut SequencerContext<Db>) -> FragV0 {
        let gas_used = in_sort.gas_used();
        self.gas_remaining -= gas_used;
        self.payment += in_sort.payment();

        let msg = FragV0::new(self.block_number, self.next_seq, in_sort.txs.iter().map(|tx| tx.tx.as_ref()), false);
        for tx in in_sort.txs {
            self.gas_used += tx.gas_used();
            let hash = tx.tx_hash();
            let logs_bloom = alloy_primitives::logs_bloom(tx.result_and_state.result.logs().iter());
            let logs = tx
                .result_and_state
                .result
                .logs()
                .iter()
                .enumerate()
                .map(|(i, t)| alloy_rpc_types::Log {
                    inner: t.clone(),
                    block_hash: None,
                    block_number: Some(self.block_number),
                    block_timestamp: Some(self.block_timestamp),
                    transaction_hash: Some(hash),
                    transaction_index: Some(self.txs.len() as u64),
                    log_index: Some(i as u64),
                    removed: false,
                })
                .collect();

            let inner_receipt = Receipt { status: Eip658Value::Eip658(true), cumulative_gas_used: self.gas_used, logs };
            let receipt = match tx.tx_type() {
                OpTxType::Legacy => OpReceiptEnvelope::Legacy(ReceiptWithBloom { receipt: inner_receipt, logs_bloom }),
                OpTxType::Eip2930 => {
                    OpReceiptEnvelope::Eip2930(ReceiptWithBloom { receipt: inner_receipt, logs_bloom })
                }
                OpTxType::Eip1559 => {
                    OpReceiptEnvelope::Eip1559(ReceiptWithBloom { receipt: inner_receipt, logs_bloom })
                }
                OpTxType::Eip7702 => {
                    OpReceiptEnvelope::Eip7702(ReceiptWithBloom { receipt: inner_receipt, logs_bloom })
                }
                OpTxType::Deposit => {
                    let inner = OpDepositReceiptWithBloom {
                        receipt: OpDepositReceipt {
                            inner: inner_receipt,
                            deposit_nonce: tx.deposit_nonce,
                            deposit_receipt_version: None,
                        },
                        logs_bloom,
                    };
                    OpReceiptEnvelope::Deposit(inner)
                }
            };
            let receipt = OpTransactionReceipt {
                inner: TransactionReceipt {
                    inner: receipt,
                    transaction_hash: hash,
                    transaction_index: Some(self.txs.len() as u64),
                    block_hash: None,
                    block_number: Some(self.block_number),
                    gas_used: tx.gas_used(),
                    effective_gas_price: tx.effective_gas_price(Some(ctx.base_fee())),
                    blob_gas_used: Some(0),
                    blob_gas_price: Some(0),
                    from: tx.sender(),
                    to: tx.to(),
                    contract_address: None,
                    authorization_list: None,
                },
                l1_block_info: L1BlockInfo::default(),
            };
            ctx.shared_state.insert_receipt(tx.tx_hash(), receipt);
            self.txs.push(tx);
        }

        self.next_seq += 1;
        self.sorting_telemetry += in_sort.telemetry;
        msg
    }

    /// Returns encoded_2718 txs, transactions root, receipts root, and receipts bloom
    pub fn encoded_txs_roots_bloom(&self, canyon_active: bool) -> (Vec<Bytes>, B256, B256, Bloom) {
        let mut receipts = Vec::with_capacity(self.txs.len());
        let mut transactions = Vec::with_capacity(self.txs.len());
        let mut logs_bloom = Bloom::ZERO;
        let mut gas_used = 0;
        for t in self.txs.iter() {
            gas_used += t.gas_used();
            let receipt = t.receipt(gas_used, canyon_active);
            logs_bloom |= receipt.logs_bloom;
            receipts.push(receipt);
            transactions.push(t.tx.encode());
        }

        let receipts_root = ordered_trie_root_with_encoder(&receipts, |r, buf| {
            r.encode_2718(buf);
        });

        let transactions_root = ordered_trie_root_with_encoder(&transactions, |tx, buf| *buf = tx.clone().into());
        (transactions, transactions_root, receipts_root, logs_bloom)
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
