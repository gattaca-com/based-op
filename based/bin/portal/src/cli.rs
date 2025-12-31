use std::{fs, path::PathBuf};

use bop_common::{
    config::{LoggingConfig, LoggingFlags},
    signing::ECDSASigner,
};
use clap::Parser;
use reqwest::Url;
use reth_rpc_layer::JwtSecret;
use tracing::level_filters::LevelFilter;

#[derive(Parser, Debug, Clone)]
#[command(version, about, name = "based-portal")]
pub struct PortalArgs {
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
    /// Signing key used to authenticate with gateways (hex string or path to file containing hex)
    #[arg(long = "gateway.signing-key")]
    pub gateway_signing_key: String,

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
    #[arg(long = "log.disable_file_logging", action = clap::ArgAction::SetFalse, default_value_t = true)]
    pub file_logging: bool,
    /// Prefix of log files
    #[arg(long = "log.prefix", default_value = "bop-portal.log")]
    pub log_prefix: String,
    /// Path for log files
    #[arg(long = "log.dir", default_value = "/tmp")]
    pub log_dir: PathBuf,
    /// Maximum number of log files
    #[arg(long = "log.max_files", default_value_t = 100)]
    pub log_max_files: usize,

    /// gateway inactivity timeout in milliseconds
    #[arg(long = "gateway.inactivity_timeout_ms", default_value_t = 3000)]
    pub gateway_timeout_inactivity_ms: u64,

    /// Enable metrics collection
    #[arg(long = "metrics.enable", default_value_t = false)]
    pub enable_metrics: bool,

    /// Port for prometheus server
    #[arg(long = "metrics.port", default_value_t = 9466)]
    pub metrics_port: u16,
}

impl PortalArgs {
    pub fn fallback_jwt(&self) -> JwtSecret {
        JwtSecret::from_hex(&self.fallback_jwt)
            .or_else(|_| JwtSecret::from_file(std::path::Path::new(&self.fallback_jwt)))
            .or_else(|_| JwtSecret::from_file(std::path::Path::new(&self.config_dir.join("jwt"))))
            .expect("Please set the --fallback.jwt flag manually, or generate and place a jwt file in the config dir")
    }

    pub fn gateway_signer(&self) -> eyre::Result<ECDSASigner> {
        parse_signing_key(&self.gateway_signing_key)
    }
}

impl From<&PortalArgs> for LoggingConfig {
    fn from(args: &PortalArgs) -> Self {
        Self {
            level: args
                .trace
                .then_some(LevelFilter::TRACE)
                .or(args.debug.then_some(LevelFilter::DEBUG))
                .unwrap_or(LevelFilter::INFO),
            flags: if args.file_logging { LoggingFlags::all() } else { LoggingFlags::StdOut },
            prefix: args.file_logging.then(|| args.log_prefix.clone()),
            max_files: args.log_max_files,
            path: args.log_dir.clone(),
            filters: None,
        }
    }
}

fn parse_signing_key(input: &str) -> eyre::Result<ECDSASigner> {
    let trimmed = input.trim();
    let normalized = trimmed.trim_start_matches("0x");
    match ECDSASigner::try_from_hex(normalized) {
        Ok(signer) => Ok(signer),
        Err(_) => {
            let contents = fs::read_to_string(trimmed)?;
            let key = contents.trim().trim_start_matches("0x");
            ECDSASigner::try_from_hex(key).map_err(|err| eyre::eyre!("failed to parse gateway signing key: {err}"))
        }
    }
}
