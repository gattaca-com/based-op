use std::{str::FromStr, time::Duration};

use alloy_provider::Provider;
use alloy_rpc_types::Header;
use bop_common::p2p::SignedVersionedMessage;
use crossbeam_channel::Sender;
use futures_util::stream::StreamExt;
use http::Uri;
use tokio_websockets::ClientBuilder;
use tracing::{error, info, warn};

use crate::types::OpRootProvider;

pub fn spawn_block_listener(provider: OpRootProvider, block_tx: Sender<Header>) {
    tokio::spawn(async move {
        loop {
            info!("Attempting to subscribe to L1 block headers...");
            let sub_result = provider.subscribe_blocks().await;

            let mut block_stream = match sub_result {
                Ok(sub) => {
                    info!("Successfully subscribed to L1 block headers.");
                    sub.into_stream()
                }
                Err(e) => {
                    error!(error = %e, "Failed to subscribe to L1 blocks, retrying in 5s");
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    continue;
                }
            };
            while let Some(header) = block_stream.next().await {
                block_tx.send(header).expect("failed to send block header");
            }
            warn!("header stream ended. Attempting to resubscribe...");
            panic!("WS connection dropped. Restart the process."); // TODO: handle reconnection
            // properly
        }
    });
}

pub fn spawn_receipt_listener_frag_stream(frag_url: &str, message_tx: Sender<SignedVersionedMessage>) {
    let frag_url = frag_url.to_string();
    tokio::spawn(async move {
        loop {
            let uri = Uri::from_str(&frag_url).expect("invalid frag stream url");
            let maybe_client = ClientBuilder::from_uri(uri).connect().await;
            let Ok((mut client, _)) = maybe_client else {
                error!("failed to connect to frag stream, reconnecting...");
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue;
            };
            while let Some(Ok(msg)) = client.next().await {
                let stream_data = msg.as_text().and_then(|s| serde_json::from_str::<SignedVersionedMessage>(s).ok());
                if let Some(msg) = stream_data {
                    message_tx.send(msg).expect("failed to send message");
                }
            }
            error!("frag stream closed, reconnecting...");
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    });
}
