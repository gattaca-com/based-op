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
use reqwest::{Client, Url};
use tokio::{runtime::Runtime, sync::oneshot};
use tracing::{info, warn};

use super::fetch_blocks::fetch_block;

#[derive(Debug)]
pub struct MockFetcher {
    /// Used to fetch blocks from an EL node.
    rpc_url: Url,
    executor: Runtime,
    next_block: u64,
    sync_until: u64,
    client: reqwest::Client,
}
impl MockFetcher {
    pub fn new(rpc_url: Url, next_block: u64, sync_until: u64) -> Self {
        let executor = tokio::runtime::Builder::new_current_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .expect("couldn't build local tokio runtime");
        let client =
            Client::builder().timeout(Duration::from_secs(5).into()).build().expect("Failed to build HTTP client");
        Self { rpc_url, executor, next_block, sync_until, client }
    }

    pub fn handle_fetch(&mut self, msg: BlockFetch) {
        match msg {
            BlockFetch::FromTo(start, stop) => {
                self.next_block = start;
                self.sync_until = stop;
            }
        }
    }
}

impl<Db: DatabaseRead> Actor<Db> for MockFetcher {
    fn on_init(&mut self, connections: &mut SpineConnections<Db>) {
        let block = self.executor.block_on(fetch_block(self.next_block, &self.client, self.rpc_url.clone()));
        let (_new_payload_status_rx, new_payload, _fcu_status_rx, fcu_1, _fcu) =
            messages::EngineApi::messages_from_block(&block, false, None);
        connections.send(new_payload);
        connections.send(fcu_1);
        self.next_block += 1;
    }

    fn loop_body(&mut self, connections: &mut SpineConnections<Db>) {
        connections.receive(|msg, _| {
            self.handle_fetch(msg);
        });
        if self.next_block < self.sync_until {
            let mut block = self.executor.block_on(fetch_block(self.next_block, &self.client, self.rpc_url.clone()));

            let (new_payload_status_rx, new_payload, fcu_status_rx, fcu_1, fcu) =
                messages::EngineApi::messages_from_block(&block, true, None);

            // let txs = Transaction::from_block(&block);
            // for t in txs {
            //     connections.send(t);
            // }
            // Duration::from_millis(100).sleep();

            connections.send(fcu);
            Duration::from_secs(1).sleep();
            let (block_tx, block_rx) = oneshot::channel();
            connections.send(EngineApi::GetPayloadV3 { payload_id: PayloadId::new([0; 8]), res: block_tx });

            let Ok(sealed_block) = block_rx.blocking_recv()  else {
                warn!("issue getting block");
                return;
            };

            // we set the extra data to 0 as that is also what the sequencer will use
            block.header.extra_data = Default::default();
            assert_eq!(sealed_block.execution_payload.payload_inner.payload_inner.block_hash, block.hash_slow(), "{block:#?} vs {sealed_block:#?}" );


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
