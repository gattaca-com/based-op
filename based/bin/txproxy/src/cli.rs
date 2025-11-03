use std::path::PathBuf;

use bop_common::config::{LoggingConfig, LoggingFlags};
use clap::Parser;
use tracing::level_filters::LevelFilter;

#[derive(Parser, Debug, Clone)]
#[command(version, about, name = "based-txproxy")]
pub struct TxProxyArgs {
    /// The port to run the txproxy on
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
    #[arg(long = "log.disable_file_logging", action = clap::ArgAction::SetFalse, default_value_t = true)]
    pub file_logging: bool,
    /// Prefix of log files
    #[arg(long = "log.prefix", default_value = "bop-txproxy.log")]
    pub log_prefix: String,
    /// Path for log files
    #[arg(long = "log.dir", default_value = "/tmp")]
    pub log_dir: PathBuf,
    /// Maximum number of log files
    #[arg(long = "log.max_files", default_value_t = 100)]
    pub log_max_files: usize,

    /// Enable metrics collection
    #[arg(long = "metrics.enable", default_value_t = false)]
    pub enable_metrics: bool,

    /// Port for prometheus server
    #[arg(long = "metrics.port", default_value_t = 9467)]
    pub metrics_port: u16,
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
            max_files: args.log_max_files,
            path: args.log_dir.clone(),
            filters: None,
        }
    }
}
