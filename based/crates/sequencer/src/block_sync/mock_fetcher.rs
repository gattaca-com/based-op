use std::sync::Arc;

use alloy_consensus::{BlockHeader, TxEip1559};
use alloy_eips::eip2718::Encodable2718;
use alloy_provider::{Provider, ProviderBuilder};
use alloy_rpc_types::engine::{ForkchoiceState, PayloadId};
use bop_common::{
    actor::Actor,
    communication::{
        messages::{self, BlockFetch, BlockSyncMessage, EngineApi},
        SpineConnections,
    },
    db::{DBFrag, DatabaseRead},
    signing::ECDSASigner,
    time::{utils::vsync_busy, Duration, Instant},
    transaction::Transaction,
};
use futures::future::join_all;
use op_alloy_consensus::OpTxEnvelope;
use op_alloy_rpc_types_engine::OpPayloadAttributes;
use reqwest::Url;
use revm_primitives::{address, b256, TxKind, U256};
use tokio::{runtime::Runtime, sync::oneshot};
use tracing::{info, warn};

use super::{
    fetch_blocks::{async_fetch_blocks_and_send_sequentially, fetch_block},
    AlloyProvider,
};

#[derive(Clone, Debug)]
pub struct BenchmarkData {
    // to be used by the mocker itself
    txs: Vec<Arc<Transaction>>,
    fcu: ForkchoiceState,
    attributes: Box<OpPayloadAttributes>,

    // config
    max_txs: usize,
    batch: u64,
    send_duration: Duration,
    get_payload_delay: Duration, //If we do 2s we will take longer due to state root
}
impl Default for BenchmarkData {
    fn default() -> Self {
        Self {
            txs: Default::default(),
            fcu: Default::default(),
            attributes: Default::default(),
            max_txs: 100_000,
            batch: 200,
            send_duration: Duration::from_millis(1400),
            get_payload_delay: Duration::from_millis(1800),
        }
    }
}

/// Different modes to run the Mocker with
///
/// Verification: Performs sequential block sync, creating `EngineApi` messages, and Txs corresponding to each block.
///               It then sends these in the right order to the Sequencer, with enough delay between the txs so they
///               hopefully get sequenced in the same order as the incoming block (otherwise we'd greedily sort them).
///               The produced block is then verified against the incoming block for equality.
/// Benchmark:    Gradually fetches more blocks in the future from the current last node in the db, until gathering up
///               a target number of (for us still) in-the-future sequenced txs. Meanwhile every 2s it sends
///               a `ForkChoiceUpdated` `EngineApi` message signalling that the `Sequencer` should start sequencing,
///               and sends the future txs over the next 2s, before sending the `GetPayload` and logging how
///               many txs were sorted and Mgas/s was reached. This process repeats on top of the current block
///               until stopped.
/// Spammer:      Will first sync as usual. Afterwards it will start spamming new txs similar to a normal node would
///               receive. No `EngineApi` messages are mocked, hence the system works as in a standard prod situation.
///               Ideally used with kurtosis
#[derive(Default, Debug, Clone)]
pub enum Mode {
    #[default]
    Verification,
    Benchmark(BenchmarkData),
    Spammer,
}

#[derive(Debug)]
pub struct MockFetcher<Db> {
    mode: Mode,
    executor: Runtime,
    next_block: u64,
    sync_until: u64,
    provider: AlloyProvider,
    db: DBFrag<Db>,
}
impl<Db> MockFetcher<Db> {
    pub fn new(rpc_url: Url, next_block: u64, sync_until: u64, db: DBFrag<Db>, mode: Mode) -> Self {
        let executor = tokio::runtime::Builder::new_current_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .expect("couldn't build local tokio runtime");
        let provider = ProviderBuilder::new().network().on_http(rpc_url);
        Self { mode, executor, next_block, sync_until, provider, db }
    }

    pub fn handle_fetch(&mut self, msg: BlockFetch) {
        if !matches!(self.mode, Mode::Spammer) {
            return;
        }
        match msg {
            BlockFetch::FromTo(start, finish) => {
                debug_assert!(start <= finish, "can't fetch with start > finish: {start} > {finish}");
                self.next_block = start.min(self.next_block);
                self.sync_until = finish.max(self.sync_until);
            }
        }
    }

    fn run_verification_body(&mut self, connections: &mut SpineConnections<Db>) {
        while connections.receive(|msg, _| {
            self.handle_fetch(msg);
        }) {}
        if self.next_block < self.sync_until {
            let mut block = self.executor.block_on(fetch_block(self.next_block, &self.provider));

            let (new_payload, fcu_1, mut fcu) = messages::EngineApi::messages_from_block(&block, true, None);

            let EngineApi::ForkChoiceUpdatedV3 { payload_attributes: Some(payload_attributes), .. } = &mut fcu else {
                unreachable!();
            };

            let txs_for_pool: Vec<_> = payload_attributes
                .transactions
                .as_mut()
                .map(|t| t.split_off(0).into_iter().map(|tx| Arc::new(Transaction::decode(tx).unwrap())).collect())
                .unwrap_or_default();
            connections.send(fcu);
            for t in txs_for_pool {
                connections.send(t);
                Duration::from_millis(10).sleep();
            }

            Duration::from_millis(2000).sleep();
            let (block_tx, mut block_rx) = oneshot::channel();
            connections.send(EngineApi::GetPayloadV3 { payload_id: PayloadId::new([0; 8]), res: block_tx });
            Duration::from_millis(100).sleep();
            let curt = Instant::now();
            let mut sealed_block = loop {
                if let Ok(sealed_block) = block_rx.try_recv() {
                    break sealed_block;
                }
                if curt.elapsed() > Duration::from_secs(2) {
                    tracing::warn!("coun't get block");
                    return;
                }
            };

            let hash = block.hash_slow();
            let hash1 = sealed_block.execution_payload.payload_inner.payload_inner.block_hash;
            if hash1 != hash {
                sealed_block.execution_payload.payload_inner.payload_inner.transactions = vec![];
                block.body = Default::default();
                let receipt = sealed_block.execution_payload.payload_inner.payload_inner.receipts_root;
                if receipt == block.receipts_root {
                    info!("receipts match");
                } else {
                    info!(our=%receipt, block = %block.receipts_root, "receipts don't match");
                    debug_assert!(false, "receipts don't match");
                };

                let gas_used = sealed_block.execution_payload.payload_inner.payload_inner.gas_used;

                if gas_used == block.gas_used() {
                    info!("gas_used matches")
                } else {
                    info!(our=%gas_used, block = %block.gas_used(), "gas_used doesn't match");
                    debug_assert!(false, "gas_used doesn't match");
                };

                let state_root = sealed_block.execution_payload.payload_inner.payload_inner.state_root;

                if state_root == block.state_root() {
                    info!("state_root matches")
                } else {
                    info!(our=%state_root, block = %block.state_root(), "state_root doesn't match");
                    debug_assert!(false, "state_root doesn't match");
                };

                println!("ACTUAL BLOCK:");
            }

            assert_eq!(
                sealed_block.execution_payload.payload_inner.payload_inner.block_hash,
                block.hash_slow(),
                "{block:#?} vs {sealed_block:#?}"
            );

            connections.send(new_payload);
            connections.send(fcu_1);

            self.next_block += 1;
        }
    }

    fn run_benchmark_body(&mut self, connections: &mut SpineConnections<Db>) {
        let Mode::Benchmark(BenchmarkData { txs, fcu, attributes, max_txs, batch, send_duration, get_payload_delay }) =
            &mut self.mode
        else {
            return;
        };

        info!("gas limit is {}", attributes.gas_limit.unwrap());
        let fcu =
            EngineApi::ForkChoiceUpdatedV3 { fork_choice_state: *fcu, payload_attributes: Some(attributes.clone()) };

        info!("sending {} txs", txs.len());
        // first we send enough for the first frag
        let curt = Instant::now();
        connections.send(fcu);
        for t in txs.iter().take(txs.len() / 10) {
            connections.send(t.clone());
        }

        if txs.len() < *max_txs {
            // if we're going to be fetching more, let's send all the rest
            for t in txs.iter().skip(txs.len() / 10) {
                connections.send(t.clone());
                // Duration::from_millis(20).sleep();
            }
            let blocks: Vec<BlockSyncMessage> = self.executor.block_on(async {
                let futures = (self.next_block..(self.next_block + *batch).min(self.sync_until))
                    .map(|i| fetch_block(i, &self.provider));
                join_all(futures).await
            });
            for b in blocks {
                for t in b.into_transactions() {
                    txs.push(Arc::new(t.into()))
                }
            }
            self.next_block += *batch;
        } else {
            let t_per_tx = *send_duration / txs.len() * 10usize / 9usize;
            for t in txs.iter().skip(txs.len() / 10) {
                vsync_busy(Some(t_per_tx), || {
                    connections.send(t.clone());
                })
            }
        }

        while curt.elapsed() < *get_payload_delay {}
        let (block_tx, block_rx) = oneshot::channel();
        connections.send(EngineApi::GetPayloadV3 { payload_id: PayloadId::new([0; 8]), res: block_tx });

        let Ok(sealed_block) = block_rx.blocking_recv() else {
            warn!("issue getting block");
            return;
        };
        let gas = sealed_block.execution_payload.payload_inner.payload_inner.gas_used;
        let n_txs = sealed_block.execution_payload.payload_inner.payload_inner.transactions.len();
        let el = curt.elapsed();
        info!(
            "in {}: sequenced {n_txs} txs, {gas} ({:.3} MGas/s)",
            curt.elapsed(),
            (gas / 1_000_000) as f64 / el.as_secs()
        );
    }
}
impl<Db: DatabaseRead> MockFetcher<Db> {
    fn run_spam_body(&mut self, connections: &mut SpineConnections<Db>) {
        while connections.receive(|msg, _| {
            self.handle_fetch(msg);
        }) {}
        if self.next_block >= self.sync_until {
            let from_account = address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266");
            let signing_wallet = ECDSASigner::try_from_secret(
                b256!("ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80").as_ref(),
            )
            .unwrap();
            let mut nonce = self.db.get_nonce(from_account).unwrap();
            let value = U256::from_limbs([1, 0, 0, 0]);
            let chain_id = 2151908;
            let to_account = address!("0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266");
            let max_gas_units = 21000;
            let max_fee_per_gas = 1_258_615_255_000;
            let max_priority_fee_per_gas = 1_000;
            for _ in 0..1000 {
                let tx = TxEip1559 {
                    chain_id,
                    nonce,
                    gas_limit: max_gas_units,
                    max_fee_per_gas,
                    max_priority_fee_per_gas,
                    to: TxKind::Call(to_account),
                    value,
                    ..Default::default()
                };
                let signed_tx = signing_wallet.sign_tx(tx).unwrap();
                let tx = OpTxEnvelope::Eip1559(signed_tx);
                let envelope = tx.encoded_2718().into();
                let tx = Arc::new(Transaction::new(tx, from_account, envelope));

                connections.send(tx);
                nonce += 1;
            }
            return;
        }
        let stop = (self.next_block + 50).min(self.sync_until);
        self.executor.block_on(async_fetch_blocks_and_send_sequentially(
            self.next_block,
            stop,
            connections.senders(),
            &self.provider,
        ));
        self.next_block = self.sync_until.min(stop + 1);
    }
}

impl<Db: DatabaseRead> Actor<Db> for MockFetcher<Db> {
    fn on_init(&mut self, connections: &mut SpineConnections<Db>) {
        let block = self.executor.block_on(fetch_block(self.next_block, &self.provider));
        let (new_payload, fcu_1, _fcu) = messages::EngineApi::messages_from_block(&block, false, None);
        connections.send(new_payload);
        connections.send(fcu_1);
        self.sync_until = self.executor.block_on(async {
            self.provider.get_block_number().await.expect("failed to fetch last block, is the RPC url correct?")
        });
        self.next_block = (self.next_block + 1).min(self.sync_until);

        let Mode::Benchmark(BenchmarkData { txs, fcu, attributes, .. }) = &mut self.mode else {
            return;
        };

        let blocks: Vec<BlockSyncMessage> = self.executor.block_on(async {
            let futures =
                (self.next_block..(self.next_block + 100).min(self.sync_until)).map(|i| fetch_block(i, &self.provider));
            join_all(futures).await
        });

        let EngineApi::ForkChoiceUpdatedV3 {
            payload_attributes: Some(mut payload_attributes), fork_choice_state, ..
        } = messages::EngineApi::messages_from_block(&blocks[0], false, None).2
        else {
            unreachable!();
        };
        *fcu = fork_choice_state;
        payload_attributes.gas_limit = payload_attributes.gas_limit.map(|t| t * 1000);
        *attributes = payload_attributes;

        for b in blocks {
            for t in b.into_transactions() {
                txs.push(Arc::new(t.into()))
            }
        }
    }

    fn loop_body(&mut self, connections: &mut SpineConnections<Db>) {
        match &mut self.mode {
            Mode::Verification => self.run_verification_body(connections),
            Mode::Benchmark(_) => {
                self.run_benchmark_body(connections);
            }
            Mode::Spammer => {
                self.run_spam_body(connections);
            }
        }
    }
}
