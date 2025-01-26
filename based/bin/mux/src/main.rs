use std::net::SocketAddr;

use axum::{
    extract::State,
    http::HeaderMap,
    response::{IntoResponse, Response},
    Json,
};
use bop_common::utils::init_tracing;
use op_alloy_rpc_types_engine::OpExecutionPayloadEnvelopeV3;
use reqwest::{Client, StatusCode, Url};
use serde::{Deserialize, Serialize};
use tracing::{info, trace};

#[tokio::main]
async fn main() {
    let _guard = init_tracing();

    info!("Hello, world!");
}

struct Mux;

struct MuxConfig {
    addr: SocketAddr,
    fallback_url: Url,
    fallback_timeout_ms: u64,
    gateway_url: Url,
    gateway_jwt: &'static str,
    gateway_timeout_ms: u64,
}

struct MuxState {
    fallback_client: Client,
    gateway_client: Client,
}

impl Mux {
    pub fn new() -> Self {
        Self
    }
}

async fn mux_request(
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
    State(state): State<MuxState>,
) -> eyre::Result<impl IntoResponse> {
    trace!(req =% body, "new_request");

    let id = body["id"].as_number().cloned().unwrap_or(serde_json::Number::from(0));
    let method = body["method"].as_str().unwrap();

    // else: send to both, return fallback

    // fork choice update: send to both, return fallback

    // new payload v3: send to gateway + return from fallback

    // get payload v3: send to fallback + send to gateway, if valid validate with fallback

    // StatusCode::OK

    Ok(StatusCode::OK)
}

#[derive(Debug, Serialize, Deserialize)]
struct JsonRpcRequest {
    id: u64,
    method: String,
    params: Option<serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn test_new_payload() {
        let p = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "eth_getBlockByNumber",
            // "params": [1, true]
        });

        let req: JsonRpcRequest = serde_json::from_value(p).unwrap();

        assert_eq!(req.id, 1);
        assert_eq!(req.method, "eth_getBlockByNumber");
        // assert_eq!(req.params, serde_json::json!([1, true]));
    }
}
