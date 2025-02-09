use std::sync::Arc;

use alloy_consensus::BlockHeader;
use alloy_provider::{Provider, ProviderBuilder};
use alloy_rpc_types::engine::PayloadId;
use bop_common::{
    actor::Actor,
    communication::{
        messages::{self, BlockFetch, EngineApi},
        SpineConnections,
    },
    db::DatabaseRead,
    time::Duration,
    transaction::Transaction,
};
use reqwest::Url;
use tokio::{runtime::Runtime, sync::oneshot};
use tracing::warn;

use super::{fetch_blocks::fetch_block, AlloyProvider};

#[derive(Debug)]
pub struct MockFetcher {
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
            .build()
            .expect("couldn't build local tokio runtime");
        let provider = ProviderBuilder::new().network().on_http(rpc_url);
        Self { executor, next_block, sync_until, provider }
    }

    pub fn handle_fetch(&mut self, msg: BlockFetch) {
        match msg {
            BlockFetch::FromTo(start, stop) => {
                // self.next_block = start;
                // self.sync_until = stop;
            }
        }
    }
}

impl<Db: DatabaseRead> Actor<Db> for MockFetcher {
    fn on_init(&mut self, connections: &mut SpineConnections<Db>) {
        let block = self.executor.block_on(fetch_block(self.next_block, &self.provider));
        let (_new_payload_status_rx, new_payload, _fcu_status_rx, fcu_1, _fcu) =
            messages::EngineApi::messages_from_block(&block, false, None);
        connections.send(new_payload);
        connections.send(fcu_1);
        self.sync_until = self.executor.block_on(async {
            self.provider.get_block_number().await.expect("failed to fetch last block, is the RPC url correct?")
        });
        tracing::info!("sync until {}", self.sync_until);

        self.next_block += 1;
    }

    fn loop_body(&mut self, connections: &mut SpineConnections<Db>) {
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
}
