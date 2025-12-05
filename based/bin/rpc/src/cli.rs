use std::path::PathBuf;

use bop_common::config::{LoggingConfig, LoggingFlags};
use clap::Parser;
use tracing::level_filters::LevelFilter;

#[derive(Parser, Debug, Clone)]
#[command(version, about, name = "based-rpc")]
pub struct RpcArgs {
    /// The port to run the rpc on
    #[arg(long = "port", default_value_t = 7545)]
    pub port: u16,

    /// ws url of the frag stream
    #[arg(long = "frag.url", default_value = "ws://0.0.0.0:9999/state_stream")]
    pub frag_url: String,

    /// ws url of eth rpc
    #[arg(long = "eth.ws.url", default_value = "ws://0.0.0.0:8546")]
    pub eth_ws_url: String,

    /// http url of eth rpc
    #[arg(long = "eth.http.url", default_value = "http://0.0.0.0:8545")]
    pub eth_http_url: String,

    /// tx receiver url
    #[arg(long = "sequencer.url")]
    pub tx_receiver_url: Option<String>,

    /// Enable debug logging
    #[arg(long)]
    pub debug: bool,
    /// Enable trace logging
    #[arg(long)]
    pub trace: bool,
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
}

impl From<&RpcArgs> for LoggingConfig {
    fn from(args: &RpcArgs) -> Self {
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
