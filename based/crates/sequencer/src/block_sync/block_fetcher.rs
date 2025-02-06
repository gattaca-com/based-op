use alloy_provider::{Provider, ProviderBuilder};
use bop_common::{
    actor::Actor,
    communication::{messages::BlockFetch, SpineConnections},
    db::DatabaseRead,
};
use reqwest::Url;
use tokio::runtime::Runtime;

use super::{fetch_blocks::async_fetch_blocks_and_send_sequentially, AlloyProvider};

#[derive(Debug)]
pub struct BlockFetcher {
    executor: Runtime,
    next_block: u64,
    sync_until: u64,
    batch_size: u64,
    provider: AlloyProvider,
}
impl BlockFetcher {
    pub fn new(rpc_url: Url) -> Self {
        let executor = tokio::runtime::Builder::new_current_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .expect("couldn't build local tokio runtime");

        let provider = ProviderBuilder::new().network().on_http(rpc_url);

        Self { executor, next_block: 0, sync_until: 0, batch_size: 20, provider }
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
    fn on_init(&mut self, _connections: &mut SpineConnections<Db>) {
        let head_block_number = self.executor.block_on(async {
            let last_block =
                self.provider.get_block_number().await.expect("failed to fetch last block, is the RPC url correct?");
            last_block
        });

        self.next_block = head_block_number;
    }

    fn loop_body(&mut self, connections: &mut SpineConnections<Db>) {
        connections.receive(|msg, _| {
            self.handle_fetch(msg);
        });
        if self.next_block < self.sync_until {
            let stop = (self.next_block + self.batch_size).min(self.sync_until);
            self.executor.block_on(async_fetch_blocks_and_send_sequentially(
                self.next_block,
                stop,
                connections.senders(),
                &self.provider,
            ));
            self.next_block = stop + 1;
        }
    }
}
