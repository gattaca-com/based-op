use bop_common::{
    communication::Producer,
    metrics::{Counter, Metric, MetricsUpdate},
    utils::uuid,
};
use futures::{FutureExt, future::BoxFuture};
use jsonrpsee::{
    MethodResponse,
    core::{ClientError, client::ClientT, traits::ToRpcParams},
    server::middleware::rpc::RpcServiceT,
    types::{ErrorObject, Params, Request, ResponsePayload, error::INTERNAL_ERROR_CODE},
};
use serde_json::value::RawValue;
use tracing::{debug, error};

use crate::clients::{AuthRpcClient, RpcClient};

#[derive(Clone)]
pub struct EngineApiProxy<S> {
    pub inner: S,
    pub geth_client: AuthRpcClient,
    pub registry_client: RpcClient,
    pub op_node_client: RpcClient,
    pub metrics: Producer<MetricsUpdate>,
}

impl<'a, S> RpcServiceT<'a> for EngineApiProxy<S>
where
    S: Send + Clone + Sync + RpcServiceT<'a> + 'a,
{
    type Future = BoxFuture<'a, MethodResponse>;

    #[tracing::instrument(skip_all, name = "middleware")]
    fn call(&self, req: Request<'a>) -> Self::Future {
        let inner = self.inner.clone();
        let method = req.method_name().to_string();
        let fallback_client = self.geth_client.clone();
        let registry_client = self.registry_client.clone();
        let op_node_client = self.op_node_client.clone();

        let uuid = uuid();
        let metrics = self.metrics;
        MetricsUpdate::send_ref(uuid, Metric::IncrementCounter(Counter::PortalTotalRequests, 1), &metrics);

        async move {
            match req.method_name().split_once('_') {
                Some(("portal", _)) => {
                    debug!(method = %method, "Received request in PortalApiProxy");
                    MetricsUpdate::send_ref(uuid, Metric::IncrementCounter(Counter::PortalApiRequests, 1), &metrics);
                    inner.call(req).await
                }
                Some(("engine", _)) => {
                    debug!(method = %method, "Received request in EngineApiProxy");
                    MetricsUpdate::send_ref(uuid, Metric::IncrementCounter(Counter::EngineApiRequests, 1), &metrics);
                    inner.call(req).await
                }
                Some(("registry", _)) => {
                    debug!(method = %method, "Received request in RegistryApiProxy");
                    MetricsUpdate::send_ref(uuid, Metric::IncrementCounter(Counter::RegistryApiRequests, 1), &metrics);
                    external_call(registry_client.clone(), &req).await
                }
                Some(("optimism", _)) | Some(("opp2p", _)) => {
                    debug!(method = %method, "Received request for OP node");
                    MetricsUpdate::send_ref(uuid, Metric::IncrementCounter(Counter::OpNodeApiRequests, 1), &metrics);
                    external_call(op_node_client.clone(), &req).await
                }
                _ => {
                    debug!(method = %method, "Forwarding request to fallback client. request: {:?}", req);
                    MetricsUpdate::send_ref(uuid, Metric::IncrementCounter(Counter::FallbackApiRequests, 1), &metrics);
                    external_call(fallback_client.clone(), &req).await
                }
            }
        }
        .boxed()
    }
}

struct WrapParams<'a>(Params<'a>);
impl ToRpcParams for WrapParams<'_> {
    fn to_rpc_params(self) -> Result<Option<Box<RawValue>>, serde_json::Error> {
        self.0.as_str().map(String::from).map(RawValue::from_string).transpose()
    }
}

async fn external_call<S>(client: S, req: &Request<'_>) -> MethodResponse
where
    S: ClientT + Send + Sync + 'static,
{
    let r: Result<serde_json::Value, jsonrpsee::core::ClientError> =
        client.request(req.method_name(), WrapParams(req.params())).await;
    match r {
        Ok(value) => {
            let payload = ResponsePayload::success(&value);
            debug!(method = %req.method_name(), "Forwarding request to client");
            MethodResponse::response(req.id.clone(), payload.into(), 4_000_000_000usize)
        }
        Err(err) => {
            error!(error = %err, "Error calling client");
            match err {
                ClientError::Call(e) => MethodResponse::error(req.id.clone(), e),
                _ => MethodResponse::error(
                    req.id.clone(),
                    ErrorObject::owned(INTERNAL_ERROR_CODE, "client error".to_string(), Some(err.to_string())),
                ),
            }
        }
    }
}
