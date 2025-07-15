use std::path::PathBuf;

use bop_common::config::{LoggingConfig, LoggingFlags};
use clap::{Parser, command};
use tracing::level_filters::LevelFilter;

#[derive(Parser, Debug, Clone)]
#[command(version, about, name = "based-portal")]
pub struct TxProxyArgs {
    /// The port to run the portal on
    #[arg(long = "port", default_value_t = 8090)]
    pub txproxy_port: u16,
    /// Enable debug logging
    #[arg(long)]
    pub debug: bool,
    /// Enable trace logging
    #[arg(long)]
    pub trace: bool,
    /// json file containing proxy urls
    #[arg(long = "tx_receivers.path")]
    pub tx_receivers_path: PathBuf,
    /// Enable file logging
    #[arg(long = "log.enable_file_logging", default_value_t = true)]
    pub file_logging: bool,
    /// Prefix of log files
    #[arg(long = "log.prefix", default_value = "bop-txproxy.log")]
    pub log_prefix: String,
}

impl From<&TxProxyArgs> for LoggingConfig {
    fn from(args: &TxProxyArgs) -> Self {
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
