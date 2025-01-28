use std::{net::SocketAddr, time::Duration};

#[derive(Debug, Clone)]
pub struct Config {
    /// Address to listen for engine_ JSON-RPC requests
    pub engine_api_addr:    SocketAddr,
    /// Internal RPC timeout to wait for engine API response
    pub engine_api_timeout: Duration,
    /// Address to listen for eth_ JSON-RPC requests
    pub eth_api_addr:       SocketAddr,
}
