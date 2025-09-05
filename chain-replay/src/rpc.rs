use std::{borrow::Cow, time::Duration};

use alloy::{
    providers::{Provider as _, RootProvider},
    rpc::types::SyncStatus,
    transports::TransportResult,
};
use alloy_network::Network;
use tokio::time::sleep;
use tracing::debug;

/// Waits for the EL client to be synced.
pub async fn wait_for_sync<N: Network>(
    provider: &RootProvider<N>,
    poll_time: Duration,
) -> TransportResult<()> {
    loop {
        sleep(poll_time).await;
        let sync_info = provider.syncing().await?;
        match sync_info {
            SyncStatus::None => return Ok(()),
            SyncStatus::Info(info) => {
                debug!("EL syncing: {info:?}");
            }
        }
    }
}

/// Reference: <https://geth.ethereum.org/docs/interacting-with-geth/rpc/ns-debug#debugsethead>
///
/// After this call, you should make a FCU to also restore the safe and finalized hash.
pub async fn debug_set_head<N: Network>(
    provider: &RootProvider<N>,
    head: u64,
) -> TransportResult<()> {
    let block_hex = format!("{:#x}", (head));
    let call = provider.raw_request::<_, ()>(Cow::from("debug_setHead"), [block_hex]);

    call.await
}
