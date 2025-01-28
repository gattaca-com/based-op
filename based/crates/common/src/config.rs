use std::net::{Ipv4Addr, SocketAddr};

use crate::time::Duration;

#[derive(Debug, Clone)]
pub struct Config {
    pub engine_api_addr: SocketAddr,
    /// Internal RPC timeout to wait for engine API response
    pub engine_api_timeout: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            engine_api_addr: SocketAddr::new(Ipv4Addr::new(0, 0, 0, 0).into(), 8001),
            engine_api_timeout: Duration::from_secs(1),
        }
    }
}
