use std::net::{Ipv4Addr, SocketAddr};

use crate::time::Duration;

#[derive(Debug, Clone)]
pub struct Config {
    /// Address to listen for engine_ JSON-RPC requests
    pub engine_api_addr: SocketAddr,
    pub eth_api_addr: SocketAddr,
    /// Internal RPC timeout to wait for engine API response
    pub engine_api_timeout: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            engine_api_addr: SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), 8001),
            eth_api_addr: SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), 8002),
            engine_api_timeout: Duration::from_secs(1),
        }
    }
}
