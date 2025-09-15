use std::path::PathBuf;

use bop_common::config::{LoggingConfig, LoggingFlags};
use clap::{Parser, command};
use tracing::level_filters::LevelFilter;

#[derive(Parser, Debug, Clone)]
#[command(version, about, name = "based-txproxy")]
pub struct TxSpammerArgs {
    /// Root wallet private key
    #[arg(long="root.private_key", default_value = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80")]
    pub root_private_key: String,
    /// Number of accounts to generate
    #[arg(long="num_accounts", default_value_t = 100)]
    pub num_accounts: usize,
    /// Throughput (tx/s)
    #[arg(long="throughput", default_value_t = 300)]
    pub throughput: usize,
    /// Funding amount per account (in ether)
    #[arg(long="funding_amount", default_value_t = 0.01)]
    pub funding_amount: f64,

    // /// Refund
    // #[arg(long="refund", default_value_t = false)]
    // pub refund: bool,

    /// Value to transfer per transaction (in ether)
    #[arg(long="tx_value", default_value_t = 0.0000000000000001)]
    pub tx_value: f64,
    /// Gas limit per transaction
    #[arg(long = "gas_limit", default_value_t = 21_000)]
    pub gas_limit: u64,
    /// Max fee per gas (in wei)
    #[arg(long = "max_fee_per_gas", default_value_t = 100_000_000)]
    pub max_fee_per_gas: u128,
    /// Max priority fee per gas (in wei)
    #[arg(long = "max_priority_fee_per_gas", default_value_t = 10)]
    pub max_priority_fee_per_gas: u128,

    /// Eth rpc url (http/ws)
    #[arg(long="eth_rpc.url", default_value = "http://127.0.0.1:8545")]
    pub eth_rpc_url: String,
    /// Sequencer URL (if specified, eth_sendRawTransaction will be sent to this URL)
    #[arg(long="sequencer.url")]
    pub sequencer_url: Option<String>,
    /// Frag stream URL (if specified, we can measure the e2e latency)
    #[arg(long="fragstream.url")]
    pub fragstream_url: Option<String>,

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

impl From<&TxSpammerArgs> for LoggingConfig {
    fn from(args: &TxSpammerArgs) -> Self {
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
