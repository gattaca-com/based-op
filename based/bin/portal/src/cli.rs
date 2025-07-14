use std::path::PathBuf;

use bop_common::config::{LoggingConfig, LoggingFlags};
use clap::{Parser, command};
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

impl PortalArgs {
    pub fn fallback_jwt(&self) -> JwtSecret {
        JwtSecret::from_hex(&self.fallback_jwt)
            .or_else(|_| JwtSecret::from_file(std::path::Path::new(&self.fallback_jwt)))
            .or_else(|_| JwtSecret::from_file(std::path::Path::new(&self.config_dir.join("jwt"))))
            .expect("Please set the --fallback.jwt flag manually, or generate and place a jwt file in the config dir")
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
            max_files: 100,
            path: PathBuf::from("/tmp"),
            filters: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::hex;
    use clap::Parser;
    use std::fs;
    use tempfile::tempdir;
    use tracing::level_filters::LevelFilter;

    #[test]
    fn parses_default_args() {
        let args = PortalArgs::parse_from(["based-portal"]);
        assert_eq!(args.portal_port, 8080);
        assert_eq!(args.config_dir, PathBuf::from("/config"));
        assert_eq!(args.fallback_eth_url, "http://0.0.0.0:8545".parse().unwrap());
        assert_eq!(args.fallback_url, "http://0.0.0.0:8551".parse().unwrap());
        assert_eq!(args.gateway_timeout_ms, 100);
        assert_eq!(args.file_logging, true);
        assert_eq!(args.log_prefix, "bop-portal.log");
    }

    #[test]
    fn parses_custom_args() {
        // file_logging == true (flag omitted, default is true)
        let args = PortalArgs::parse_from([
            "based-portal",
            "--port",
            "1234",
            "--config-dir",
            "/tmp/config",
            "--fallback.eth_url",
            "http://localhost:9999",
            "--log.prefix",
            "custom.log",
        ]);
        assert_eq!(args.portal_port, 1234);
        assert_eq!(args.config_dir, PathBuf::from("/tmp/config"));
        assert_eq!(args.fallback_eth_url, "http://localhost:9999".parse().unwrap());
        assert_eq!(args.file_logging, true);
        assert_eq!(args.log_prefix, "custom.log");

        // file_logging == true (flag present)
        let args = PortalArgs::parse_from(["based-portal", "--log.enable_file_logging"]);
        assert_eq!(args.file_logging, true);
    }

    #[test]
    fn logging_config_from_args() {
        // file_logging == true (flag omitted, default is true)
        let args = PortalArgs::parse_from(["based-portal", "--log.prefix", "test.log", "--trace"]);
        let config = LoggingConfig::from(&args);
        assert!(config.flags.contains(LoggingFlags::all()));
        assert_eq!(config.prefix, Some("test.log".to_string()));
        assert_eq!(config.level, LevelFilter::TRACE);

        // file_logging == true (flag present)
        let args = PortalArgs::parse_from([
            "based-portal",
            "--log.enable_file_logging",
            "--log.prefix",
            "test.log",
            "--trace",
        ]);
        let config = LoggingConfig::from(&args);
        assert!(config.flags.contains(LoggingFlags::all()));
        assert_eq!(config.prefix, Some("test.log".to_string()));
        assert_eq!(config.level, LevelFilter::TRACE);
    }

    #[test]
    fn fallback_jwt_from_hex() {
        // 32 bytes hex string (64 chars)
        let hex_str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let args = PortalArgs::parse_from(["based-portal", "--fallback.jwt", hex_str]);
        let jwt = args.fallback_jwt();
        assert_eq!(hex::encode(jwt.as_bytes()), hex_str);
    }

    #[test]
    fn fallback_jwt_from_file() {
        let dir = tempdir().unwrap();
        let jwt_path = dir.path().join("jwt");
        let jwt_bytes = [0x42u8; 32];
        fs::write(&jwt_path, hex::encode(jwt_bytes)).unwrap();
        let args = PortalArgs::parse_from(["based-portal", "--fallback.jwt", jwt_path.to_str().unwrap()]);
        let jwt = args.fallback_jwt();
        assert_eq!(jwt.as_bytes(), &jwt_bytes);
    }

    #[test]
    fn fallback_jwt_from_config_dir() {
        let dir = tempdir().unwrap();
        let jwt_path = dir.path().join("jwt");
        let jwt_bytes = [0x99u8; 32];
        fs::write(&jwt_path, hex::encode(jwt_bytes)).unwrap();
        let args = PortalArgs::parse_from([
            "based-portal",
            "--config-dir",
            dir.path().to_str().unwrap(),
            "--fallback.jwt",
            "not_a_real_file",
        ]);
        let jwt = args.fallback_jwt();
        assert_eq!(jwt.as_bytes(), &jwt_bytes);
    }

    #[test]
    #[should_panic(expected = "Please set the --fallback.jwt flag manually")]
    fn fallback_jwt_fails_if_missing() {
        let dir = tempdir().unwrap();
        let args = PortalArgs::parse_from([
            "based-portal",
            "--config-dir",
            dir.path().to_str().unwrap(),
            "--fallback.jwt",
            "not_a_real_file",
        ]);
        // No jwt file, not a hex string, should panic
        let _ = args.fallback_jwt();
    }

    #[test]
    #[should_panic]
    fn fallback_jwt_fails_on_invalid_hex() {
        let args = PortalArgs::parse_from(["based-portal", "--fallback.jwt", "not_hex"]);
        let _ = args.fallback_jwt();
    }
}
