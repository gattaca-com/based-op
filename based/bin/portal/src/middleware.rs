use futures::{future::BoxFuture, FutureExt};
use jsonrpsee::{
    core::{client::ClientT, traits::ToRpcParams, TEN_MB_SIZE_BYTES},
    server::middleware::rpc::RpcServiceT,
    types::{
        error::{INTERNAL_ERROR_CODE, INTERNAL_ERROR_MSG},
        ErrorObject, Params, Request, ResponsePayload,
    },
    MethodResponse,
};
use serde_json::value::RawValue;
use tracing::debug;

use crate::server::HttpClient;

#[derive(Clone)]
pub struct ProxyService<S> {
    supported_methods: &'static [&'static str],
    inner: S,
    fallback_client: HttpClient,
}

impl<S> ProxyService<S> {
    pub fn new(supported_methods: &'static [&'static str], inner: S, fallback_client: HttpClient) -> Self {
        Self { supported_methods, inner, fallback_client }
    }
}

impl<'a, S> RpcServiceT<'a> for ProxyService<S>
where
    S: Send + Clone + Sync + RpcServiceT<'a> + 'a,
{
    type Future = BoxFuture<'a, MethodResponse>;

    #[tracing::instrument(skip_all, name = "middleware")]
    fn call(&self, req: Request<'a>) -> Self::Future {
        let inner = self.inner.clone();
        let fallback_client = self.fallback_client.clone();
        let supported_methods = self.supported_methods;

        async move {
            if supported_methods.contains(&req.method_name()) {
                // debug!(method = %req.method_name(), "handling request");

                inner.call(req).await
            } else {
                debug!(method = %req.method_name(), "forwarding request to fallback");

                let params = WrapParams(req.params());

                let r: Result<serde_json::Value, jsonrpsee::core::ClientError> =
                    fallback_client.request(req.method_name(), params).await;

                match r {
                    Ok(r) => {
                        let payload = ResponsePayload::success(r);
                        MethodResponse::response(req.id, payload.into(), TEN_MB_SIZE_BYTES as usize)
                    }
                    Err(_) => MethodResponse::error(
                        req.id,
                        ErrorObject::borrowed(INTERNAL_ERROR_CODE, INTERNAL_ERROR_MSG, None),
                    ),
                }
            }
        }
        .boxed()
    }
}

// TODO: remove this
struct WrapParams<'a>(Params<'a>);
impl ToRpcParams for WrapParams<'_> {
    fn to_rpc_params(self) -> Result<Option<Box<RawValue>>, serde_json::Error> {
        // FIXME: we should not clone here
        self.0.as_str().map(String::from).map(RawValue::from_string).transpose()
    }
}

#[cfg(test)]
mod tests {
    use std::{
        net::SocketAddr,
        sync::{atomic::AtomicBool, Arc},
    };

    use jsonrpsee::{
        http_client::{HttpClient, HttpClientBuilder},
        server::{RpcServiceBuilder, ServerBuilder},
        RpcModule,
    };
    use reth_rpc_layer::{AuthClientLayer, JwtSecret};

    use super::*;

    #[tokio::test]
    async fn test_proxy() {
        let received_fallback = Arc::new(AtomicBool::new(false));
        let received_mux = Arc::new(AtomicBool::new(false));

        let fallback_server =
            ServerBuilder::default().build("127.0.0.1:8080".parse::<SocketAddr>().unwrap()).await.unwrap();
        let mut module = RpcModule::new(());
        let rcv = received_fallback.clone();
        module
            .register_method("hello_fallback", move |_, _, _| {
                rcv.store(true, std::sync::atomic::Ordering::Relaxed);
            })
            .unwrap();

        let _fallback_handle = fallback_server.start(module);

        let secret_layer = AuthClientLayer::new(JwtSecret::random());
        let middleware = tower::ServiceBuilder::default().layer(secret_layer);
        let fallback_client =
            HttpClientBuilder::default().set_http_middleware(middleware).build("http://127.0.0.1:8080").unwrap();

        let rpc_middleware =
            RpcServiceBuilder::new().layer_fn(move |s| ProxyService::new(&["hello_mux"], s, fallback_client.clone()));

        let mux_server = ServerBuilder::default()
            .set_rpc_middleware(rpc_middleware)
            .build("127.0.0.1:8081".parse::<SocketAddr>().unwrap())
            .await
            .unwrap();
        let mut mux_module = RpcModule::new(());

        let rcv = received_mux.clone();
        mux_module
            .register_method("hello_mux", move |_, _, _| {
                rcv.store(true, std::sync::atomic::Ordering::Relaxed);
            })
            .unwrap();

        let _mux_server_handle = mux_server.start(mux_module);

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let client = HttpClient::builder().build("http://127.0.0.1:8081").unwrap();

        let _: serde_json::Value = client.request("hello_mux", vec![""]).await.unwrap();
        assert!(received_mux.load(std::sync::atomic::Ordering::Relaxed));

        let _: serde_json::Value = client.request("hello_fallback", vec![""]).await.unwrap();
        assert!(received_fallback.load(std::sync::atomic::Ordering::Relaxed));
    }
}
