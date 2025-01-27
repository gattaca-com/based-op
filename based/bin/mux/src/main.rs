use std::{borrow::Cow, net::SocketAddr, sync::Arc, time::Duration};

use alloy_rpc_types::engine::{ForkchoiceUpdated, PayloadStatus};
use axum::{
    extract::State,
    http::{Extensions, HeaderMap},
    response::IntoResponse,
    routing::post,
    Json, Router,
};
use bop_common::utils::{init_tracing, wait_for_signal};
use error::{MuxError, RpcResult};
use jsonrpsee::types::{Id, Request};
use op_alloy_rpc_types_engine::OpExecutionPayloadEnvelopeV3;
use reqwest::{header, Client, StatusCode, Url};
use serde_json::{self};
use tokio::net::TcpListener;
use tracing::{debug, error, info, trace, Level};

mod error;

#[tokio::main]
async fn main() {
    let config = get_config();

    let _guard = init_tracing();

    match run(config).await {
        Ok(_) => info!("mux exited"),
        Err(err) => {
            eprintln!("mux exited with error: {err}");
            error!(%err, "mux exited with error")
        }
    }
}

async fn run(config: MuxConfig) -> eyre::Result<()> {
    let mut headers = HeaderMap::new();
    let mut auth_value = header::HeaderValue::try_from(format!("Bearer {}", config.gateway_jwt)).unwrap();
    auth_value.set_sensitive(true);
    headers.insert(header::AUTHORIZATION, auth_value);

    let gateway_client = reqwest::ClientBuilder::new()
        .timeout(Duration::from_millis(config.gateway_timeout_ms))
        .default_headers(headers)
        .build()
        .unwrap();

    let fallback_client =
        reqwest::ClientBuilder::new().timeout(Duration::from_millis(config.fallback_timeout_ms)).build().unwrap();

    let addr = config.addr;
    let state = MuxState { fallback_client, gateway_client, config: Arc::new(config) };

    let app = Router::new().route("/", post(mux_request)).with_state(state);

    info!(%addr, "starting Engine API mux");
    let listener = TcpListener::bind(addr).await?;

    tokio::spawn(async move {
        if let Err(err) = axum::serve(listener, app).await {
            error!(%err, "mux server exited");
        };
    });

    wait_for_signal().await
}

fn get_config() -> MuxConfig {
    todo!()
}

#[derive(Clone)]
struct MuxConfig {
    addr: SocketAddr,
    fallback_url: Url,
    fallback_timeout_ms: u64,
    gateway_url: Url,
    gateway_jwt: &'static str,
    gateway_timeout_ms: u64,
}

#[derive(Clone)]
struct MuxState {
    fallback_client: Client,
    gateway_client: Client,
    config: Arc<MuxConfig>,
}

const FORKCHOICE_METHOD: &str = "engine_forkchoiceUpdatedV3";
const NEW_PAYLOAD_METHOD: &str = "engine_newPayloadV3";
const GET_PAYLOAD_METHOD: &str = "engine_getPayloadV3";

// forkchoice: with attributes - start building, otherwise just sync
// new_payload: validate payload
// get_-paylod: get a new execution payload

#[tracing::instrument(skip_all, name = "mux", fields(id =% uuid::Uuid::new_v4()), err, ret(level = Level::DEBUG))]
async fn mux_request(
    headers: HeaderMap,
    State(state): State<MuxState>,
    Json(req): Json<serde_json::Value>,
) -> Result<impl IntoResponse, MuxError> {
    trace!(?req, "new request");

    let method = req["method"].as_str().unwrap_or_default();
    debug!(method, "new request");

    match method {
        FORKCHOICE_METHOD => {
            // params: ForkchoiceState, Option<OpPayloadAttributes>
            // returns: ForkchoiceUpdated

            // send in background to gateway
            // return what fallback returns

            tokio::spawn({
                let req = req.clone();
                let client = state.gateway_client.clone();
                let url = state.config.gateway_url.clone();

                async move {
                    match send_rpc_request::<ForkchoiceUpdated>(client, url, req, None).await {
                        Ok(res) => {
                            if res.is_valid() {
                                debug!(?res, "gateway forkchoice");
                            } else {
                                error!(?res, "gateway forkchoice");
                            }
                        }
                        Err(err) => {
                            error!(%err, "failed gateway forkchoice");
                        }
                    }
                }
            });

            let response = send_rpc_request::<ForkchoiceUpdated>(
                state.fallback_client,
                state.config.fallback_url.clone(),
                req,
                Some(headers),
            )
            .await?;

            Ok((StatusCode::OK, Json(response)).into_response())
        }

        GET_PAYLOAD_METHOD => {
            // params: PayloadId
            // returns: OpExecutionPayloadEnvelopeV3

            // send to fallback and gateway, if gateway returns, validate with new_paylaod and
            // return otherwise return what fallback returns

            let fallback_payload = tokio::spawn({
                let req = req.clone();
                let client = state.fallback_client.clone();
                let url = state.config.fallback_url.clone();

                send_rpc_request::<OpExecutionPayloadEnvelopeV3>(client, url, req, Some(headers.clone()))
            });

            let gateway_payload = tokio::spawn({
                let req = req.clone();
                let client = state.gateway_client.clone();
                let url = state.config.gateway_url.clone();

                let fallback_client = state.fallback_client.clone();
                let fallback_url = state.config.fallback_url.clone();

                async move {
                    let gateway_payload =
                        send_rpc_request::<OpExecutionPayloadEnvelopeV3>(client, url, req, None).await?;

                    let params = serde_json::value::to_raw_value(&[
                        serde_json::to_value(gateway_payload.execution_payload.clone())?,
                        serde_json::to_value(Vec::<()>::new())?,
                        serde_json::to_value(gateway_payload.parent_beacon_block_root)?,
                    ])?;
                    let params = Cow::Owned(params);

                    // validate with fallback
                    let request = Request {
                        jsonrpc: jsonrpsee::types::TwoPointZero,
                        id: Id::Number(0),
                        method: NEW_PAYLOAD_METHOD.into(),
                        params: Some(params),
                        extensions: Extensions::new(),
                    };

                    let request = serde_json::to_value(request)?;

                    let gateway_status =
                        send_rpc_request::<PayloadStatus>(fallback_client, fallback_url, request, Some(headers))
                            .await?;

                    if gateway_status.is_valid() {
                        debug!(?gateway_payload, ?gateway_status, "gateway get_payload");
                        Ok(gateway_payload)
                    } else {
                        error!(?gateway_payload, ?gateway_status, "gateway get_payload");
                        Err(MuxError::Internal)
                    }
                }
            });

            let (fallback, gateway) = tokio::join!(fallback_payload, gateway_payload);

            // ignore join errors
            let fallback = fallback?;
            let gateway = gateway?;

            let payload = gateway.or(fallback)?;

            Ok((StatusCode::OK, Json(payload)).into_response())
        }

        NEW_PAYLOAD_METHOD => {
            // params: ExecutionPayloadV3, Vec<B256>, B256
            // returns: PayloadStatus

            tokio::spawn({
                let req = req.clone();
                let client = state.gateway_client.clone();
                let url = state.config.gateway_url.clone();

                async move {
                    match send_rpc_request::<PayloadStatus>(client, url, req, None).await {
                        Ok(res) => {
                            if res.is_valid() {
                                debug!(?res, "gateway new_payload");
                            } else {
                                error!(?res, "gateway new_payload");
                            }
                        }
                        Err(err) => {
                            error!(%err, "failed gateway new_payload");
                        }
                    }
                }
            });

            let response = send_rpc_request::<PayloadStatus>(
                state.fallback_client,
                state.config.fallback_url.clone(),
                req,
                Some(headers),
            )
            .await?;

            Ok((StatusCode::OK, Json(response)).into_response())
        }

        _ => {
            // send to both, return to fallback

            // Send to gateway in background
            tokio::spawn({
                let req = req.clone();
                let client = state.gateway_client.clone();
                let url = state.config.gateway_url.clone();

                async move {
                    if let Err(err) = send_rpc_request::<serde_json::Value>(client, url, req, None).await {
                        error!(%err, "failed gateway request");
                    }
                }
            });

            // Send to fallback and return its response
            let response = send_rpc_request::<serde_json::Value>(
                state.fallback_client,
                state.config.fallback_url.clone(),
                req,
                Some(headers),
            )
            .await?;

            Ok((StatusCode::OK, Json(response)).into_response())
        }
    }
}

async fn send_rpc_request<T: serde::de::DeserializeOwned>(
    client: Client,
    url: Url,
    req: serde_json::Value,
    headers: Option<HeaderMap>,
) -> Result<T, MuxError> {
    let mut response = client.post(url);

    if let Some(headers) = headers {
        response = response.headers(headers);
    }

    let response = response.json(&req).send().await?;

    let body = response.json::<serde_json::Value>().await?;
    let body: RpcResult<T> = serde_json::from_value(body)?;
    let body = body?; // rpc error

    Ok(body)
}
