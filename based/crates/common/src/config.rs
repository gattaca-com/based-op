use std::net::SocketAddr;

use reqwest::Url;

use crate::time::Duration;

#[derive(Debug, Clone)]
pub struct Config {
    /// Address to listen for engine_ JSON-RPC requests
    pub engine_api_addr: SocketAddr,
    pub eth_api_addr: SocketAddr,
    /// Internal RPC timeout to wait for engine API response
    pub engine_api_timeout: Duration,
    /// Url to full node RPC (temp)
    pub eth_fallback_url: Url,
}
