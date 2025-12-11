use bop_common::time::Duration;
use jsonrpsee::{
    core::middleware::layer::RpcLogger,
    http_client::{HttpClient, HttpClientBuilder, RpcService, transport::HttpBackend},
};
use reqwest::Url;
use reth_rpc_layer::{AuthClientLayer, AuthClientService, JwtSecret};

mod gateway;
pub use gateway::*;

pub type RpcClient = jsonrpsee::http_client::HttpClient;
pub fn create_client(url: Url, timeout: Duration) -> eyre::Result<RpcClient> {
    let client = HttpClientBuilder::default()
        .max_request_size(u32::MAX)
        .max_response_size(u32::MAX)
        .request_timeout(timeout.into())
        .build(url)?;
    Ok(client)
}

pub type AuthRpcClient = HttpClient<RpcLogger<RpcService<AuthClientService<HttpBackend>>>>;
pub fn create_auth_client(url: Url, token: JwtSecret, timeout: Duration) -> eyre::Result<AuthRpcClient> {
    let secret_layer = AuthClientLayer::new(token);
    let middleware = tower::ServiceBuilder::default().layer(secret_layer);

    let client = HttpClientBuilder::default()
        .max_request_size(u32::MAX)
        .max_response_size(u32::MAX)
        .set_http_middleware(middleware)
        .request_timeout(timeout.into())
        .build(url)?;

    Ok(client)
}
