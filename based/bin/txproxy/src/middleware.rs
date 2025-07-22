use std::sync::Arc;

use futures::{FutureExt, future::BoxFuture};
use jsonrpsee::{
    MethodResponse,
    core::{client::ClientT, traits::ToRpcParams},
    http_client::HttpClient,
    server::middleware::rpc::RpcServiceT,
    types::{ErrorObject, Request, ResponsePayload, error::INTERNAL_ERROR_CODE},
};
use parking_lot::RwLock;
use serde_json::value::RawValue;
use tracing::{debug, error, info};

#[derive(Clone)]
pub struct MultiplexingService {
    forwarding_to: Arc<RwLock<Vec<HttpClient>>>,
}

impl MultiplexingService {
    pub fn new(forwarding_to: Arc<RwLock<Vec<HttpClient>>>) -> Self {
        Self { forwarding_to }
    }
}

impl<'a> RpcServiceT<'a> for MultiplexingService {
    type Future = BoxFuture<'a, MethodResponse>;

    #[tracing::instrument(skip_all, name = "middleware")]
    fn call(&self, req: Request<'a>) -> Self::Future {
        //TODO: is this really the best way to do this?
        let forwarding_to_arc = Arc::clone(&self.forwarding_to);

        let method = req.method_name().to_string();
        let params_raw = req.params.unwrap().get().to_string();

        async move {
            info!(method = %method, params = %params_raw, "Received request for method");

            if method != "eth_sendRawTransaction" {
                error!(method = %method, "Unsupported method for multiplexing: {}", method);
                return MethodResponse::error(
                    req.id,
                    ErrorObject::owned(
                        INTERNAL_ERROR_CODE,
                        "Method not supported".to_string(),
                        Some("Only eth_sendRawTransaction is supported".to_string()),
                    ),
                );
            }

            let clients_to_forward = forwarding_to_arc.read().clone();

            if clients_to_forward.is_empty() {
                debug!("No forwarding clients available");
                return MethodResponse::error(
                    req.id,
                    ErrorObject::owned(
                        INTERNAL_ERROR_CODE,
                        "No forwarding clients available".to_string(),
                        Some("Please check your configuration".to_string()),
                    ),
                );
            }

            let request_tasks = clients_to_forward
                .into_iter()
                .map(|client| {
                    let method = method.clone();
                    let params = WrapParams::from_raw(params_raw.clone());
                    async move {
                        let r: Result<serde_json::Value, jsonrpsee::core::ClientError> =
                            client.request(method.as_str(), params).await;
                        match r {
                            Ok(value) => {
                                debug!(response = ?value, "Request processed successfully (individual)");
                                Ok(value)
                            }
                            Err(e) => {
                                debug!(error = %e, "Error while processing request (individual)");
                                Err(e)
                            }
                        }
                    }
                    .boxed()
                })
                .collect::<Vec<_>>();

            let responses = futures::future::select_ok(request_tasks).await;

            match responses {
                Ok((value, other_tasks)) => {
                    let payload = ResponsePayload::success(&value);
                    tokio::spawn(async move {
                        futures::future::join_all(other_tasks).await;
                    });
                    info!(response = ?payload, "Request processed successfully");
                    MethodResponse::response(req.id, payload.into(), 4_000_000_000usize)
                }
                Err(e) => {
                    error!(error = %e, "All clients failed to process request");
                    MethodResponse::error(
                        req.id,
                        ErrorObject::owned(INTERNAL_ERROR_CODE, "Internal error".to_string(), Some(e.to_string())),
                    )
                }
            }
        }
        .boxed()
    }
}

// TODO: remove this
#[derive(Debug, Clone)]
struct WrapParams(String);

impl WrapParams {
    fn from_raw(params_raw: String) -> Self {
        Self(params_raw)
    }
}

impl ToRpcParams for WrapParams {
    fn to_rpc_params(self) -> Result<Option<Box<RawValue>>, serde_json::Error> {
        Ok(Some(RawValue::from_string(self.0).unwrap()))
    }
}
