use std::sync::Arc;

use futures::{FutureExt, future::BoxFuture};
use jsonrpsee::{
    MethodResponse,
    core::{client::ClientT, traits::ToRpcParams},
    http_client::HttpClient,
    server::middleware::rpc::RpcServiceT,
    types::{
        ErrorObject, Request, ResponsePayload,
        error::{INTERNAL_ERROR_CODE, INVALID_PARAMS_CODE, METHOD_NOT_FOUND_CODE},
    },
};
use parking_lot::RwLock;
use serde_json::value::RawValue;
use tracing::{debug, error};

use crate::server::FlowCounter;

#[derive(Clone)]
pub struct MultiplexingService {
    forwarding_to: Arc<RwLock<Vec<HttpClient>>>,
    flow_counter: Arc<FlowCounter>,
}

impl MultiplexingService {
    pub fn new(forwarding_to: Arc<RwLock<Vec<HttpClient>>>, flow_counter: Arc<FlowCounter>) -> Self {
        Self { forwarding_to, flow_counter }
    }
}

impl<'a> RpcServiceT<'a> for MultiplexingService {
    type Future = BoxFuture<'a, MethodResponse>;

    #[tracing::instrument(skip_all, name = "middleware")]
    fn call(&self, req: Request<'a>) -> Self::Future {
        let forwarding_to_arc = Arc::clone(&self.forwarding_to);
        let flow_counter = Arc::clone(&self.flow_counter);
        flow_counter.increment_total();
        
        async move {
            let method = req.method_name().to_string();
            let Some(params_raw) = req.params else {
                flow_counter.increment_failed_invalid_params();
                return MethodResponse::error(
                    req.id,
                    ErrorObject::owned(INVALID_PARAMS_CODE, "Invalid request".to_string(), None::<()>),
                );
            };
            let params_raw = params_raw.get().to_string();

            debug!(%method, params = %params_raw, "Received request for method");

            if method != "eth_sendRawTransaction" {
                debug!(%method, "Unsupported method for multiplexing");
                flow_counter.increment_failed_method_not_found();
                return MethodResponse::error(
                    req.id,
                    ErrorObject::owned(
                        METHOD_NOT_FOUND_CODE,
                        "Method not supported".to_string(),
                        Some("Only eth_sendRawTransaction is supported".to_string()),
                    ),
                );
            }

            let clients_to_forward = forwarding_to_arc.read().clone();

            if clients_to_forward.is_empty() {
                debug!("No forwarding clients available");
                flow_counter.increment_failed_no_clients();
                return MethodResponse::error(
                    req.id,
                    ErrorObject::owned(
                        INTERNAL_ERROR_CODE,
                        "No forwarding clients available".to_string(),
                        Some("Please check your configuration".to_string()),
                    ),
                );
            }

            let request_tasks = clients_to_forward.iter().map(|client| {
                let client = client.clone();
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
                        Err(err) => {
                            debug!(%err, "Error while processing request (individual)");
                            Err(err)
                        }
                    }
                }
                .boxed()
            });

            let responses = futures::future::select_ok(request_tasks).await;

            match responses {
                Ok((value, other_tasks)) => {
                    let payload = ResponsePayload::success(&value);
                    tokio::spawn(futures::future::join_all(other_tasks));
                    debug!(response = ?payload, "Request processed successfully");
                    flow_counter.increment_success();
                    MethodResponse::response(req.id, payload.into(), 4_000_000_000usize)
                }
                Err(err) => {
                    error!(%err, "All clients failed to process request");
                    flow_counter.increment_failed_all_clients();
                    MethodResponse::error(
                        req.id,
                        ErrorObject::owned(INTERNAL_ERROR_CODE, "Internal error".to_string(), Some(err.to_string())),
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
