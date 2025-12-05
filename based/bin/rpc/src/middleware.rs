use futures::FutureExt;
use jsonrpsee::{
    MethodResponse,
    core::{
        ClientError,
        client::ClientT,
        middleware::{Batch, Notification},
        traits::ToRpcParams,
    },
    server::middleware::rpc::RpcServiceT,
    types::{ErrorObject, Params, Request, ResponsePayload, error::INTERNAL_ERROR_CODE},
};
use serde_json::value::RawValue;
use tracing::{debug, error};

pub type RpcClient = jsonrpsee::http_client::HttpClient;

#[derive(Clone)]
pub struct EthApiProxy<S> {
    pub inner: S,
    pub geth_client: RpcClient,
}

const SUPPORTED_METHODS: &[&str] = &[
    "eth_sendRawTransaction",
    "eth_sendRawTransactionSync",
    "eth_getTransactionReceipt",
    // "eth_getBlockByNumber",
    // "eth_getBlockByHash",
    "eth_blockNumber",
    "eth_getTransactionCount",
    "eth_getBalance",
    "eth_call",
];

impl<S> RpcServiceT for EthApiProxy<S>
where
    S: RpcServiceT<MethodResponse = MethodResponse> + Send + Sync + Clone + 'static,
{
    type BatchResponse = S::BatchResponse;
    type MethodResponse = S::MethodResponse;
    type NotificationResponse = S::NotificationResponse;

    #[tracing::instrument(skip_all, name = "middleware")]
    fn call<'a>(&self, req: Request<'a>) -> impl Future<Output = Self::MethodResponse> + Send + 'a {
        let inner = self.inner.clone();
        let fallback_client = self.geth_client.clone();

        async move {
            if SUPPORTED_METHODS.contains(&req.method_name()) {
                inner.call(req).await
            } else {
                external_call(fallback_client.clone(), &req).await
            }
        }
        .boxed()
    }

    fn batch<'a>(&self, batch: Batch<'a>) -> impl Future<Output = Self::BatchResponse> + Send + 'a {
        self.inner.batch(batch)
    }

    fn notification<'a>(&self, n: Notification<'a>) -> impl Future<Output = Self::NotificationResponse> + Send + 'a {
        self.inner.notification(n)
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
