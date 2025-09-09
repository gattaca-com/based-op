use std::{borrow::Cow, time::Duration};

use alloy::{
    providers::{Provider as _, RootProvider},
    transports::TransportResult,
};
use alloy_network::Network;
use tokio::time::sleep;
use tracing::debug;

/// Waits for the EL client to be on a certain head.
pub async fn wait_for_el_on_head<N: Network>(
    provider: &RootProvider<N>,
    target: u64,
) -> TransportResult<()> {
    loop {
        sleep(Duration::from_secs(5)).await;
        let block_number = provider.get_block_number().await?;
        if block_number == target {
            break;
        } else {
            debug!(
                head = block_number,
                target, "Waiting for based-op-geth to be on the correct head"
            );
        }
    }
    Ok(())
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
