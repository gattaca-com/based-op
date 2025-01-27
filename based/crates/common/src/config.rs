use std::{net::SocketAddr, time::Duration};

#[derive(Debug, Clone)]
pub struct Config {
    pub engine_api_addr: SocketAddr,
    /// Internal RPC timeout to wait for engine API response
    pub engine_api_timeout: Duration,
}
