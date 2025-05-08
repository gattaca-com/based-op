use futures::{FutureExt, future::BoxFuture};
use jsonrpsee::{
    MethodResponse,
    core::{client::ClientT, traits::ToRpcParams},
    server::middleware::rpc::RpcServiceT,
    types::{
        ErrorObject, Params, Request, ResponsePayload,
        error::{INTERNAL_ERROR_CODE, INTERNAL_ERROR_MSG},
    },
};
use serde_json::value::RawValue;
use tracing::{debug, error};

use crate::server::{AuthRpcClient, RpcClient};

#[derive(Clone)]
pub struct ProxyService<S> {
    supported_methods: &'static [&'static str],
    inner: S,
    fallback_eth_client: RpcClient,
    fallback_client: AuthRpcClient,
    op_client: RpcClient,
    registry_client: RpcClient,
}

impl<S> ProxyService<S> {
    pub fn new(
        supported_methods: &'static [&'static str],
        inner: S,
        fallback_eth_client: RpcClient,
        fallback_client: AuthRpcClient,
        op_client: RpcClient,
        registry_client: RpcClient,
    ) -> Self {
        Self { supported_methods, inner, fallback_eth_client, fallback_client, op_client, registry_client }
    }
}

impl<'a, S> RpcServiceT<'a> for ProxyService<S>
where
    S: Send + Clone + Sync + RpcServiceT<'a> + 'a,
{
    type Future = BoxFuture<'a, MethodResponse>;

    #[tracing::instrument(skip_all, name = "middleware")]
    fn call(&self, req: Request<'a>) -> Self::Future {
        //TODO: is this really the best way to do this?
        let inner = self.inner.clone();
        let fallback_client = self.fallback_client.clone();
        let fallback_eth_client = self.fallback_eth_client.clone();
        let registry_client = self.registry_client.clone();
        let op_client = self.op_client.clone();

        let supported_methods = self.supported_methods;

        async move {
            if &req.method_name()[0..=5] == "portal" || supported_methods.contains(&req.method_name()) {
                debug!(method = %req.method_name(), "handling request");
                inner.call(req).await
            } else {
                let params = WrapParams(req.params());
                let r: Result<serde_json::Value, jsonrpsee::core::ClientError> = match req.method_name().split_once('_')
                {
                    Some(("engine", _)) => {
                        debug!(method = %req.method_name(), "forwarding request to fallback");
                        fallback_client.clone().request(req.method_name(), params).await
                    }
                    Some(("eth", _)) => {
                        debug!(method = %req.method_name(), "forwarding request to eth fallback");
                        fallback_eth_client.clone().request(req.method_name(), params).await
                    }
                    Some(("optimism", _)) | Some(("opp2p", _)) => {
                        debug!(method = %req.method_name(), "forwarding request to op-node fallback");
                        op_client.request(req.method_name(), params).await
                    }

                    Some(("node", rest)) => {
                        debug!(method = %req.method_name(), "forwarding request to op-node");
                        op_client.request(rest, params).await
                    }

                    Some(("geth", rest)) => {
                        debug!(method = %req.method_name(), "forwarding request to op-geth");
                        fallback_client.clone().request(rest, params).await
                    }

                    Some(("registry", rest)) => {
                        debug!(method = %req.method_name(), "forwarding request to registry");
                        registry_client.request(req.method_name(), params).await
                    }
                    Some((_, _)) => {
                        error!(method = %req.method_name(), "i don't know what to do with this");
                        Err(jsonrpsee::core::ClientError::Custom("method nonexistent".to_string()))
                    }
                    None => {
                        error!(id = %req.id ,"received empty request");
                        Err(jsonrpsee::core::ClientError::Custom("empty request".to_string()))
                    }
                };

                match r {
                    Ok(r) => {
                        let payload = ResponsePayload::success(r);
                        MethodResponse::response(req.id, payload.into(), 4_000_000_000usize)
                    }
                    Err(err) => {
                        error!(?err, "error forwarding {} request", req.method_name());

                        MethodResponse::error(
                            req.id,
                            ErrorObject::borrowed(INTERNAL_ERROR_CODE, INTERNAL_ERROR_MSG, None),
                        )
                    }
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
        sync::{Arc, atomic::AtomicBool},
    };

    use jsonrpsee::{
        RpcModule,
        http_client::{HttpClient, HttpClientBuilder},
        server::{RpcServiceBuilder, ServerBuilder},
    };
    use reth_rpc_layer::{AuthClientLayer, JwtSecret};

    use super::*;

    #[ignore = "Requires RPC calls"]
    #[tokio::test]
    async fn test_proxy() {
        let received_fallback = Arc::new(AtomicBool::new(false));
        let received_eth_fallback = Arc::new(AtomicBool::new(false));
        let received_mux = Arc::new(AtomicBool::new(false));

        let fallback_server = ServerBuilder::default()
            .max_request_body_size(u32::MAX)
            .max_response_body_size(u32::MAX)
            .build("127.0.0.1:9090".parse::<SocketAddr>().unwrap())
            .await
            .unwrap();
        let mut module = RpcModule::new(());
        let rcv = received_fallback.clone();
        module
            .register_method("engine_fallback", move |_, _, _| {
                rcv.store(true, std::sync::atomic::Ordering::Relaxed);
            })
            .unwrap();

        let _fallback_handle = fallback_server.start(module);

        let eth_fallback_server = ServerBuilder::default()
            .max_request_body_size(u32::MAX)
            .max_response_body_size(u32::MAX)
            .build("127.0.0.1:9091".parse::<SocketAddr>().unwrap())
            .await
            .unwrap();
        let mut eth_module = RpcModule::new(());
        let rcv = received_eth_fallback.clone();
        eth_module
            .register_method("eth_fallback", move |_, _, _| {
                rcv.store(true, std::sync::atomic::Ordering::Relaxed);
            })
            .unwrap();

        let _eth_fallback_handle = eth_fallback_server.start(eth_module);

        let secret_layer = AuthClientLayer::new(JwtSecret::random());
        let middleware = tower::ServiceBuilder::default().layer(secret_layer);
        let fallback_client = HttpClientBuilder::default()
            .max_request_size(u32::MAX)
            .max_response_size(u32::MAX)
            .set_http_middleware(middleware)
            .build("http://127.0.0.1:9090")
            .unwrap();

        let fallback_eth_client = HttpClientBuilder::default()
            .max_request_size(u32::MAX)
            .max_response_size(u32::MAX)
            .build("http://127.0.0.1:9091")
            .unwrap();

        let op_client = HttpClientBuilder::default()
            .max_request_size(u32::MAX)
            .max_response_size(u32::MAX)
            .build("http://127.0.0.1:9545")
            .unwrap();
        let registry_client = HttpClientBuilder::default()
            .max_request_size(u32::MAX)
            .max_response_size(u32::MAX)
            .build("http://127.0.0.1:8081")
            .unwrap();

        let rpc_middleware = RpcServiceBuilder::new().layer_fn(move |s| {
            ProxyService::new(
                &["hello_mux"],
                s,
                fallback_eth_client.clone(),
                fallback_client.clone(),
                op_client.clone(),
                registry_client.clone(),
            )
        });

        let mux_server = ServerBuilder::default()
            .max_request_body_size(u32::MAX)
            .max_response_body_size(u32::MAX)
            .set_rpc_middleware(rpc_middleware)
            .build("127.0.0.1:9092".parse::<SocketAddr>().unwrap())
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

        let client = HttpClient::builder()
            .max_request_size(u32::MAX)
            .max_response_size(u32::MAX)
            .build("http://127.0.0.1:9092")
            .unwrap();

        let _: serde_json::Value = client.request("hello_mux", vec![""]).await.unwrap();
        assert!(received_mux.load(std::sync::atomic::Ordering::Relaxed));

        let _: serde_json::Value = client.request("engine_fallback", vec![""]).await.unwrap();
        assert!(received_fallback.load(std::sync::atomic::Ordering::Relaxed));

        let _: serde_json::Value = client.request("eth_fallback", vec![""]).await.unwrap();
        assert!(received_eth_fallback.load(std::sync::atomic::Ordering::Relaxed));
    }
}
