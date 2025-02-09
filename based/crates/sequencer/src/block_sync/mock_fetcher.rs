use std::sync::Arc;

use alloy_consensus::BlockHeader;
use alloy_provider::{Provider, ProviderBuilder};
use alloy_rpc_types::engine::{ForkchoiceState, PayloadId};
use bop_common::{
    actor::Actor,
    communication::{
        messages::{self, BlockFetch, BlockSyncMessage, EngineApi},
        SpineConnections,
    },
    db::DatabaseRead,
    time::{utils::vsync_busy, Duration, Instant},
    transaction::Transaction,
};
use core_affinity::CoreId;
use futures::future::join_all;
use op_alloy_rpc_types_engine::OpPayloadAttributes;
use reqwest::Url;
use tokio::{runtime::Runtime, sync::oneshot};
use tracing::warn;

use super::{fetch_blocks::fetch_block, AlloyProvider};
#[derive(Default, Debug, Clone)]
enum Mode {
    #[default]
    Verification,
    Benchmark(Vec<Arc<Transaction>>, ForkchoiceState, Box<OpPayloadAttributes>),
}

#[derive(Debug)]
pub struct MockFetcher {
    mode: Mode,
    executor: Runtime,
    next_block: u64,
    sync_until: u64,
    provider: AlloyProvider,
}
impl MockFetcher {
    pub fn new(rpc_url: Url, next_block: u64, sync_until: u64) -> Self {
        let executor = tokio::runtime::Builder::new_current_thread()
            .worker_threads(1)
            .enable_all()
            .on_thread_start(|| {
                core_affinity::set_for_current(CoreId { id: 1 });
            })
            .build()
            .expect("couldn't build local tokio runtime");
        let provider = ProviderBuilder::new().network().on_http(rpc_url);
        Self {
            mode: Mode::default(),
            // mode: Mode::Benchmark(vec![], Default::default(), Default::default()),
            executor,
            next_block,
            sync_until,
            provider,
        }
    }

    pub fn handle_fetch(&mut self, msg: BlockFetch) {
        match msg {
            BlockFetch::FromTo(_start, _stop) => {
                // self.next_block = start;
                // self.sync_until = stop;
            }
        }
    }

    fn run_verification_body<Db>(&mut self, connections: &mut SpineConnections<Db>) {
        connections.receive(|msg, _| {
            self.handle_fetch(msg);
        });
        if self.next_block < self.sync_until {
            let mut block = self.executor.block_on(fetch_block(self.next_block, &self.provider));

            let (_new_payload_status_rx, new_payload, _fcu_status_rx, fcu_1, mut fcu) =
                messages::EngineApi::messages_from_block(&block, true, None);

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
                Duration::from_millis(20).sleep();
            }

            Duration::from_millis(2000).sleep();
            let (block_tx, block_rx) = oneshot::channel();
            connections.send(EngineApi::GetPayloadV3 { payload_id: PayloadId::new([0; 8]), res: block_tx });

            let Ok(mut sealed_block) = block_rx.blocking_recv() else {
                warn!("issue getting blocq");
                return;
            };

            let hash = block.hash_slow();
            let hash1 = sealed_block.execution_payload.payload_inner.payload_inner.block_hash;
            if hash1 != hash {
                sealed_block.execution_payload.payload_inner.payload_inner.transactions = vec![];
                block.body = Default::default();
                let receipt = sealed_block.execution_payload.payload_inner.payload_inner.receipts_root;
                if receipt == block.receipts_root {
                    tracing::info!("receipts match");
                } else {
                    tracing::info!(our=%receipt, block = %block.receipts_root, "receipts don't match");
                    debug_assert!(false, "receipts don't match");
                };

                let gas_used = sealed_block.execution_payload.payload_inner.payload_inner.gas_used;

                if gas_used == block.gas_used() {
                    tracing::info!("gas_used matches")
                } else {
                    tracing::info!(our=%gas_used, block = %block.gas_used(), "gas_used doesn't match");
                    debug_assert!(false, "gas_used doesn't match");
                };

                let state_root = sealed_block.execution_payload.payload_inner.payload_inner.state_root;

                if state_root == block.state_root() {
                    tracing::info!("state_root matches")
                } else {
                    tracing::info!(our=%state_root, block = %block.state_root(), "state_root doesn't match");
                    debug_assert!(false, "state_root doesn't match");
                };

                // println!("OUR BLOCK:");
                // println!("{sealed_block:#?}");
                println!("ACTUAL BLOCK:");
                // println!("{block:#?}");
                // panic!("block hash mismatch {hash} vs {hash1}");
            }

            assert_eq!(
                sealed_block.execution_payload.payload_inner.payload_inner.block_hash,
                block.hash_slow(),
                "{block:#?} vs {sealed_block:#?}"
            );

            connections.send(new_payload);
            connections.send(fcu_1);

            // let Ok(r) = new_payload_status_rx.blocking_recv() else {
            //     tracing::error!("issue with getting payload status");
            //     return;
            // };
            // tracing::info!("got {r:?} status for new_payload_status, sending fcu");

            // let Ok(r) = fcu_status_rx.blocking_recv() else {
            //     tracing::error!("issue with getting payload status");
            //     return;
            // };
            // tracing::info!("got {r:?} status for fcu");

            self.next_block += 1;
        }
    }

    fn run_benchmark_body<Db>(&mut self, connections: &mut SpineConnections<Db>) {
        let (rx, _tx) = oneshot::channel();
        let Mode::Benchmark(txs, fcu, op_attributes) = &mut self.mode else {
            return;
        };

        tracing::info!("gas limit is {}", op_attributes.gas_limit.unwrap());
        let fcu = EngineApi::ForkChoiceUpdatedV3 {
            fork_choice_state: *fcu,
            payload_attributes: Some(op_attributes.clone()),
            res_tx: rx,
        };

        tracing::info!("sending {} txs", txs.len());
        // first we send enough for the first frag
        let curt = Instant::now();
        connections.send(fcu);
        for t in txs.iter().take(txs.len() / 10) {
            connections.send(t.clone());
        }

        if txs.len() < 100_000 {
            // if we're going to be fetching more, let's send all the rest
            for t in txs.iter().skip(txs.len() / 10) {
                connections.send(t.clone());
                // Duration::from_millis(20).sleep();
            }
            let blocks: Vec<BlockSyncMessage> = self.executor.block_on(async {
                let futures = (self.next_block..(self.next_block + 200).min(self.sync_until))
                    .map(|i| fetch_block(i, &self.provider));
                join_all(futures).await
            });
            for b in blocks {
                for t in b.into_transactions() {
                    txs.push(Arc::new(t.into()))
                }
            }
            self.next_block += 200;
        } else {
            let t_per_tx = Duration::from_millis(1800) / txs.len() * 10usize / 9usize;
            for t in txs.iter().skip(txs.len() / 10) {
                vsync_busy(Some(t_per_tx), || {
                    connections.send(t.clone());
                })
            }
        }

        while curt.elapsed() < Duration::from_millis(2000) {}
        let (block_tx, block_rx) = oneshot::channel();
        connections.send(EngineApi::GetPayloadV3 { payload_id: PayloadId::new([0; 8]), res: block_tx });

        let Ok(sealed_block) = block_rx.blocking_recv() else {
            warn!("issue getting blocq");
            return;
        };
        let gas = sealed_block.execution_payload.payload_inner.payload_inner.gas_used;
        let n_txs = sealed_block.execution_payload.payload_inner.payload_inner.transactions.len();
        let el = curt.elapsed();
        tracing::info!(
            "in {}: sequenced {n_txs} txs, {gas} ({} MGas/s)",
            curt.elapsed(),
            (gas / 1_000_000) as f64 / el.as_secs()
        );
    }
}

impl<Db: DatabaseRead> Actor<Db> for MockFetcher {
    const CORE_AFFINITY: Option<usize> = Some(1);

    fn on_init(&mut self, connections: &mut SpineConnections<Db>) {
        let block = self.executor.block_on(fetch_block(self.next_block, &self.provider));
        let (_new_payload_status_rx, new_payload, _fcu_status_rx, fcu_1, _fcu) =
            messages::EngineApi::messages_from_block(&block, false, None);
        connections.send(new_payload);
        connections.send(fcu_1);
        self.sync_until = self.executor.block_on(async {
            self.provider.get_block_number().await.expect("failed to fetch last block, is the RPC url correct?")
        });
        self.next_block += 1;

        let Mode::Benchmark(bench_txs, bench_forkchoice_state, bench_op_attributes) = &mut self.mode else {
            return;
        };

        let blocks: Vec<BlockSyncMessage> = self.executor.block_on(async {
            let futures =
                (self.next_block..(self.next_block + 100).min(self.sync_until)).map(|i| fetch_block(i, &self.provider));
            join_all(futures).await
        });

        let EngineApi::ForkChoiceUpdatedV3 {
            payload_attributes: Some(mut payload_attributes), fork_choice_state, ..
        } = messages::EngineApi::messages_from_block(&blocks[0], false, None).4
        else {
            unreachable!();
        };
        *bench_forkchoice_state = fork_choice_state;
        payload_attributes.gas_limit = payload_attributes.gas_limit.map(|t| t * 1000);
        *bench_op_attributes = payload_attributes;

        for b in blocks {
            for t in b.into_transactions() {
                bench_txs.push(Arc::new(t.into()))
            }
        }
    }

    fn loop_body(&mut self, connections: &mut SpineConnections<Db>) {
        match &mut self.mode {
            Mode::Verification => self.run_verification_body(connections),
            Mode::Benchmark(_, _, _) => {
                self.run_benchmark_body(connections);
            }
        }
    }
}
