use axum::{
    extract::{State, WebSocketUpgrade, ws::Message},
    response::Response,
};
use bop_common::p2p::SignedVersionedMessage;
use tokio::sync::broadcast::error::RecvError;
use tracing::warn;

#[derive(Clone)]
pub(crate) struct StreamState {
    pub frags_tx: tokio::sync::broadcast::Sender<SignedVersionedMessage>,
}

/// Handles WS stream for frags with state
pub(crate) async fn state_stream(state: State<StreamState>, ws: WebSocketUpgrade) -> Result<Response, ()> {
    let mut frags_rx = state.frags_tx.subscribe();

    Ok(ws.on_upgrade(async move |mut socket: axum::extract::ws::WebSocket| {
        loop {
            tokio::select! {
                frag = frags_rx.recv() => {
                    let frag = match frag {
                        Ok(frag) => frag,
                        Err(RecvError::Lagged(_)) => {
                            continue;
                        }
                        Err(RecvError::Closed) => {
                            break;
                        }
                    };


                    let text = serde_json::to_string(&frag).unwrap();
                    if let Err(err) = socket.send(Message::text(text)).await {
                        warn!(?err, "failed to send frag message, closing");
                        break;
                    }
                }

                Some(msg) = socket.recv() => {
                    match msg {
                        Ok(Message::Ping(payload)) => {
                            if socket.send(Message::Pong(payload)).await.is_err() {
                                break;
                            }
                        }

                        Ok(Message::Close(_)) => {
                            break;
                        }

                        _ => {
                            continue;
                        }
                    }
                }
            }
        }
    }))
}
