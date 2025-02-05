use std::time::Duration;

use bop_common::{
    actor::Actor,
    communication::{messages::BlockFetch, SpineConnections},
    db::DatabaseRead,
};
use reqwest::{Client, Url};
use tokio::runtime::Runtime;

use super::fetch_blocks::async_fetch_blocks_and_send_sequentially;

#[derive(Debug)]
pub struct BlockFetcher {
    /// Used to fetch blocks from an EL node.
    rpc_url: Url,
    executor: Runtime,
    next_block: u64,
    sync_until: u64,
    batch_size: u64,
    client: reqwest::Client,
}
impl BlockFetcher {
    pub fn new(rpc_url: Url) -> Self {
        let executor = tokio::runtime::Builder::new_current_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .expect("couldn't build local tokio runtime");
        let client = Client::builder().timeout(Duration::from_secs(5)).build().expect("Failed to build HTTP client");
        Self { rpc_url, executor, next_block: 0, sync_until: 0, batch_size: 20, client }
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

impl<Db: DatabaseRead> Actor<Db> for BlockFetcher {
    fn loop_body(&mut self, connections: &mut SpineConnections<Db>) {
        connections.receive(|msg, _| {
            self.handle_fetch(msg);
        });
        if self.next_block < self.sync_until {
            let stop = (self.next_block + self.batch_size).min(self.sync_until);
            self.executor.block_on(async_fetch_blocks_and_send_sequentially(
                self.next_block,
                stop,
                self.rpc_url.clone(),
                connections.senders(),
                &self.client,
            ));
            self.next_block = stop + 1;
        }
    }
}
