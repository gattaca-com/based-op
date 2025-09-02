use std::{net::Ipv4Addr, path::PathBuf, sync::Arc};

use alloy_rpc_types::engine::JwtSecret;
use clap::Parser;
use reqwest::Url;
use reth_cli::chainspec::ChainSpecParser;
use reth_optimism_chainspec::OpChainSpec;
use reth_optimism_cli::chainspec::OpChainSpecParser;
use revm_primitives::B256;
use strum_macros::EnumString;
use tracing::level_filters::LevelFilter;

#[derive(Clone, Debug, EnumString)]
pub enum MockMode {
    Benchmark,
    Spammer,
    Verification,
}

#[derive(Parser, Debug)]
#[command(version, about, name = "gateway")]
pub struct GatewayArgs {
    #[arg(
        long,
        value_name = "CHAIN_OR_PATH",
        long_help = OpChainSpecParser::help_message(),
        value_parser = OpChainSpecParser::parser(),
    )]
    pub chain: Arc<OpChainSpec>,
    /// The host to run the engine_ and eth_ RPC
    #[arg(long = "rpc.host", default_value_t = Ipv4Addr::UNSPECIFIED)]
    pub rpc_host: Ipv4Addr,
    /// The port to run the engine_ and eth_ RPC
    #[arg(long = "rpc.port", default_value_t = 9997)]
    pub rpc_port: u16,
    #[arg(long = "rpc.port_no_auth", default_value_t = 9998)]
    pub rpc_port_no_auth: u16,
    #[arg(long = "rpc.jwt")]
    pub rpc_jwt: String,
    /// Url to an L2 eth api rpc
    #[arg(long = "eth_client.url", default_value = "http://localhost:8545")]
    pub eth_client_url: Url,
    /// Url to the root peer gossip node
    #[arg(long = "gossip.root_peer_url", default_value = "http://localhost:8547")]
    pub gossip_root_peer_url: Url,
    /// Gossip to sign frag messages
    #[arg(long = "gossip.signer_private_key")]
    pub gossip_signer_private_key: Option<B256>,
    /// Duration of a frag in ms
    #[arg(long = "sequencer.frag_duration_ms", default_value_t = 200)]
    pub frag_duration_ms: u64,
    /// Number of sims per loop
    #[arg(long = "sequencer.sim_threads", default_value_t = 5)]
    pub sim_threads: usize,
    /// Database location
    #[arg(long = "db.datadir")]
    pub db_datadir: PathBuf,
    /// Maximum number of cached accounts
    #[arg(long = "db.max_cached_accounts", default_value_t = 10_000)]
    pub max_cached_accounts: u64,
    /// Maximum number of cached storages
    #[arg(long = "db.max_cached_storages", default_value_t = 100_000)]
    pub max_cached_storages: u64,
    /// Mock mode
    #[arg(long = "mock")]
    pub mock: Option<MockMode>,
    #[clap(flatten)]
    pub replay_args: Option<ReplayArgs>,
    /// Enable DEBUG logging
    #[arg(long = "debug")]
    pub debug: bool,
    /// Enable TRACE logging
    #[arg(long = "trace")]
    pub trace: bool,
    /// Enable file logging
    #[arg(long = "log.enable_file_logging", default_value_t = true)]
    pub file_logging: bool,
    /// Prefix of log files
    #[arg(long = "log.prefix", default_value = "bop-gateway.log")]
    pub log_prefix: String,
    /// Maximum number of log files
    #[arg(long = "log.max_files", default_value_t = 30)]
    pub log_max_files: usize,
    /// Path for log files
    #[arg(long = "log.dir", default_value = "/tmp")]
    pub log_dir: PathBuf,
    /// Add additional filters for logging
    #[arg(long = "log.filters")]
    pub log_filters: Option<String>,
    /// If true will commit locally sequenced blocks to the db before getting payload from the engine api.
    #[arg(long = "sequencer.commit_sealed_frags_to_db", default_value_t = false)]
    pub commit_sealed_frags_to_db: bool,
    /// URL of the supervisor service for transaction validation
    #[arg(long = "supervisor.url")]
    pub supervisor_url: Option<Url>,
    /// URL of the supervisor service for transaction validation
    #[arg(
        long = "supervisor.safety_level",
        help = "Safety level to pass to supervisor, values: finalized, safe, local-safe, cross-unsafe, unsafe, invalid"
    )]
    pub supervisor_safety_level: Option<String>,

    /// Simulator Allows reverts in simulations
    #[arg(long = "simulator.allow_reverts", default_value_t = true)]
    pub allow_reverts: bool,
}

impl GatewayArgs {
    pub fn sequencer_jwt(&self) -> JwtSecret {
        JwtSecret::from_hex(&self.rpc_jwt)
            .or_else(|_| JwtSecret::from_file(std::path::Path::new(&self.rpc_jwt)))
            .expect("Couldn't parse sequencer_jwt")
    }
}

bitflags::bitflags! {
    #[repr(transparent)]
    #[derive(Clone, Copy, Debug, PartialEq, PartialOrd, Ord, Eq)]
    pub struct LoggingFlags: u8 {
        const File   = 0b00000001;
        const StdOut = 0b00000010;
    }
}
impl Default for LoggingFlags {
    fn default() -> Self {
        Self::StdOut | Self::File
    }
}

#[derive(Debug, Clone)]
pub struct LoggingConfig {
    pub level: LevelFilter,
    pub flags: LoggingFlags,
    pub prefix: Option<String>,
    pub max_files: usize,
    pub path: PathBuf,
    pub filters: Option<String>,
}

impl LoggingConfig {
    pub fn default_with_prefix(prefix: String) -> Self {
        Self {
            prefix: Some(prefix),
            level: LevelFilter::INFO,
            flags: LoggingFlags::default(),
            max_files: 100,
            path: PathBuf::from("/tmp"),
            filters: None,
        }
    }
}

impl From<&GatewayArgs> for LoggingConfig {
    fn from(args: &GatewayArgs) -> Self {
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
            filters: args.log_filters.clone(),
        }
    }
}

/// Arguments required for running the gateway in chain-replication mode. See [`crates::sequencer::block_sync::ReplayFetcher`] for
/// more details.
#[derive(Debug, Clone, Parser)]
pub struct ReplayArgs {
    #[clap(long = "chain-replay.l2-el-verifier-url")]
    pub l2_el_verifier_url: Url,
    /// The block number which expresses the range of blocks we want to replay. In particular,
    /// assuming the local eth client is at height N <= replay_target, the chain-replication will
    /// happen for range N..=replay_target.
    #[clap(long = "chain-replay.target")]
    pub replay_target: u64,
}
