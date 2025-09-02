use alloy_consensus::BlockHeader;
use alloy_provider::Provider as _;
use alloy_rpc_types::engine::PayloadId;
use bop_common::{
    actor::Actor,
    communication::{
        SpineConnections,
        messages::{self, BlockFetch, EngineApi},
    },
    db::DatabaseRead,
    time::{Duration, Instant},
    transaction::Transaction,
};
use reqwest::Url;
use tokio::{runtime::Runtime, sync::oneshot};
use tracing::{debug, info};

use super::{
    AlloyProvider,
    fetch_blocks::{async_fetch_blocks_and_send_sequentially, fetch_block},
};

/// A block fetcher specifically intended for chain-replation. The replay fetcher make the gateway
/// sync up to the specified the specified target, and then will start replay the chain by
/// injecting transactions of the subsequent blocks as if they came from the RPC, verifying its
/// correctness.
///
/// During replication, it Performs sequential block sync, creating `EngineApi` messages, and txs
/// corresponding to each block. It then sends these in the right order to the Sequencer, with
/// enough delay between the txs so they hopefully get sequenced in the same order as the incoming
/// block (otherwise we'd greedily sort them). The produced block is then verified against the
/// incoming block for equality.
///
/// TODO: fix this docs.
#[derive(Debug)]
pub struct ReplayFetcher {
    executor: Runtime,
    next_block: u64,
    sync_until: u64,
    replay_target: u64,
    batch_size: u64,
    /// The local provider which expresses the local view of the chain, at a shorter height.
    local_provider: AlloyProvider,
    /// The verification provider from which we'll download blocks to replay.
    verification_provider: AlloyProvider,
}
impl ReplayFetcher {
    pub fn new(db_block: u64, local_provider_url: Url, verification_provider_url: Url, replay_target: u64) -> Self {
        let executor = tokio::runtime::Builder::new_current_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .expect("couldn't build local tokio runtime");

        let local_provider = AlloyProvider::new_http(local_provider_url);
        let verification_provider = AlloyProvider::new_http(verification_provider_url);

        Self {
            executor,
            next_block: db_block + 1,
            sync_until: db_block + 1,
            replay_target,
            batch_size: 20,
            local_provider,
            verification_provider,
        }
    }

    pub fn handle_fetch(&mut self, _msg: BlockFetch) {
        // match msg {
        //     BlockFetch::FromTo(start, stop) => {
        //         self.next_block = start.min(self.next_block);
        //         self.sync_until = stop.max(self.sync_until);
        //     }
        // }
    }

    /// After the local chain is synced up to `self.sync_until`, start replaying the chain by
    /// sequencing and verifying we end up in the same state.
    fn run_verification_body<Db: DatabaseRead>(&mut self, connections: &mut SpineConnections<Db>) {
        while connections.receive(|msg, _| {
            self.handle_fetch(msg);
        }) {}

        let transactions_in_attributes = false;
        let no_tx_pool = Some(true);

        if self.next_block < self.replay_target {
            let head_block_number = self.next_block.saturating_sub(1);

            let prev_block = self.executor.block_on(fetch_block(head_block_number, &self.verification_provider));
            debug!("Fetched previous block {} for newPayload + FCU", prev_block.number());

            let (new_payload_prev_block, _, fcu) =
                messages::EngineApi::messages_from_block(&prev_block, transactions_in_attributes, no_tx_pool);

            let block_to_build = self.executor.block_on(fetch_block(self.next_block, &self.verification_provider));

            debug!("Fetched block {} to replay", block_to_build.number());

            // WaitingForNewPayload -> WaitingForForkchoiceWithAttributes
            connections.send(new_payload_prev_block);
            Duration::from_millis(1000).sleep();

            let txs_for_pool: Vec<_> = Transaction::from_block(&block_to_build);

            // WaitingForForkchoiceWithAttributes -> Sorting
            connections.send(fcu);

            for t in txs_for_pool {
                connections.send(t);
                Duration::from_millis(10).sleep();
            }

            Duration::from_millis(2000).sleep();

            let (block_tx, mut block_rx) = oneshot::channel();
            // Sorting -> WaitingForNewPayload
            connections.send(EngineApi::GetPayloadV4 { payload_id: PayloadId::new([0; 8]), res: block_tx });
            Duration::from_millis(100).sleep();
            let curt = Instant::now();
            let mut sealed_block = loop {
                if let Ok(sealed_block) = block_rx.try_recv() {
                    break sealed_block;
                }
                if curt.elapsed() > Duration::from_secs(2) {
                    tracing::warn!("couldn't get block");
                    return;
                }
            };

            let hash = block_to_build.hash_slow();
            let hash1 = sealed_block.execution_payload.payload_inner.payload_inner.payload_inner.block_hash;
            if hash1 != hash {
                sealed_block.execution_payload.payload_inner.payload_inner.payload_inner.transactions = vec![];
                let receipt = sealed_block.execution_payload.payload_inner.payload_inner.payload_inner.receipts_root;
                if receipt == block_to_build.receipts_root {
                    info!("receipts match");
                } else {
                    info!(our=%receipt, block = %block_to_build.receipts_root, "receipts don't match");
                    debug_assert!(false, "receipts don't match");
                };

                let gas_used = sealed_block.execution_payload.payload_inner.payload_inner.payload_inner.gas_used;

                if gas_used == block_to_build.gas_used() {
                    info!("gas_used matches")
                } else {
                    info!(our=%gas_used, block = %block_to_build.gas_used(), "gas_used doesn't match");
                    debug_assert!(false, "gas_used doesn't match");
                };

                let state_root = sealed_block.execution_payload.payload_inner.payload_inner.payload_inner.state_root;

                if state_root == block_to_build.state_root() {
                    info!("state_root matches")
                } else {
                    info!(our=%state_root, block = %block_to_build.state_root(), "state_root doesn't match");
                    debug_assert!(false, "state_root doesn't match");
                };

                // println!("ACTUAL BLOCK:");
            }

            assert_eq!(
                sealed_block.execution_payload.payload_inner.payload_inner.payload_inner.block_hash,
                block_to_build.hash_slow(),
                "{block_to_build:#?} vs {sealed_block:#?}"
            );

            // WaitingForNewPayload -> WaitingForForkchoiceWithAttributes
            // connections.send(new_payload);
            // connections.send(fcu_1);

            self.next_block += 1;
        }
    }
}

impl<Db: DatabaseRead> Actor<Db> for ReplayFetcher {
    fn on_init(&mut self, _connections: &mut SpineConnections<Db>) {
        let local_head = self.executor.block_on(async {
            self.local_provider.get_block_number().await.expect("failed to fetch last block, is the RPC url correct?")
        });

        self.sync_until = local_head;
    }

    fn loop_body(&mut self, connections: &mut SpineConnections<Db>) {
        if self.next_block < self.sync_until {
            let stop = (self.next_block + self.batch_size).min(self.sync_until);
            self.executor.block_on(async_fetch_blocks_and_send_sequentially(
                self.next_block,
                stop,
                connections.senders(),
                &self.verification_provider,
            ));
            self.next_block = stop + 1;
        } else {
            // We're done with syncing, we can start replaying.
            self.run_verification_body(connections);
        }

        while connections.receive(|msg, _| {
            self.handle_fetch(msg);
        }) {}
    }
}
