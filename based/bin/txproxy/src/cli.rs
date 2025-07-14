use std::path::PathBuf;

use bop_common::config::{LoggingConfig, LoggingFlags};
use clap::{Parser, command};
use reqwest::Url;
use reth_rpc_layer::JwtSecret;
use tracing::level_filters::LevelFilter;

#[derive(Parser, Debug, Clone)]
#[command(version, about, name = "based-portal")]
pub struct TxProxy {
    /// Config dir where rollup.json and genesis.json can be found
    #[arg(short, long, default_value = "/config")]
    pub config_dir: PathBuf,

    /// The port to run the portal on
    #[arg(long = "port", default_value_t = 8080)]
    pub portal_port: u16,

    /// TEMP: the URL to the based-op-node's RPC-API
    #[arg(long = "op_node.url", default_value = "http://0.0.0.0:8547")]
    pub op_node_url: Url,

    /// TEMP: the URL to the fallback EthAPI
    #[arg(long = "fallback.eth_url", default_value = "http://0.0.0.0:8545")]
    pub fallback_eth_url: Url,

    /// The URL to the fallback EngineAPI
    #[arg(long = "fallback.engine_url", default_value = "http://0.0.0.0:8551")]
    pub fallback_url: Url,

    /// Timeout for fallback requests in milliseconds
    #[arg(long = "fallback.timeout_ms", default_value_t = 60_000)]
    pub fallback_timeout_ms: u64,

    /// The JWT token to use for the fallback
    #[arg(long = "fallback.jwt", default_value = "/config/jwt")]
    pub fallback_jwt: String,

    /// Timeout for gateway requests in milliseconds
    #[arg(long = "gateway.timeout_ms", default_value_t = 100)]
    pub gateway_timeout_ms: u64,

    /// Enable debug logging
    #[arg(long)]
    pub debug: bool,

    /// Enable trace logging
    #[arg(long)]
    pub trace: bool,

    /// port where the registry is running
    #[arg(long = "registry.url", default_value = "http://0.0.0.0:8081")]
    pub registry_url: Url,

    #[arg(long = "registry.timeout_ms", default_value_t = 100)]
    pub registry_timeout_ms: u64,
    /// Enable file logging
    #[arg(long = "log.enable_file_logging", default_value_t = true)]
    pub file_logging: bool,
    /// Prefix of log files
    #[arg(long = "log.prefix", default_value = "bop-portal.log")]
    pub log_prefix: String,

    /// gateway inactivity timeout in milliseconds
    #[arg(long = "gateway.inactivity_timeout_ms", default_value_t = 3000)]
    pub gateway_timeout_inactivity_ms: u64,
}

impl TxProxy {
    pub fn fallback_jwt(&self) -> JwtSecret {
        JwtSecret::from_hex(&self.fallback_jwt)
            .or_else(|_| JwtSecret::from_file(std::path::Path::new(&self.fallback_jwt)))
            .or_else(|_| JwtSecret::from_file(std::path::Path::new(&self.config_dir.join("jwt"))))
            .expect("Please set the --fallback.jwt flag manually, or generate and place a jwt file in the config dir")
    }
}

impl From<&TxProxy> for LoggingConfig {
    fn from(args: &TxProxy) -> Self {
        Self {
            level: args
                .trace
                .then_some(LevelFilter::TRACE)
                .or(args.debug.then_some(LevelFilter::DEBUG))
                .unwrap_or(LevelFilter::INFO),
            flags: if args.file_logging { LoggingFlags::all() } else { LoggingFlags::StdOut },
            prefix: args.file_logging.then(|| args.log_prefix.clone()),
            max_files: 100,
            path: PathBuf::from("/tmp"),
            filters: None,
        }
    }
}
