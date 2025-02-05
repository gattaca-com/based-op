use std::{net::Ipv4Addr, path::PathBuf, sync::Arc};

use alloy_primitives::Address;
use clap::Parser;
use reqwest::Url;
use reth_cli::chainspec::ChainSpecParser;
use reth_optimism_chainspec::OpChainSpec;
use reth_optimism_cli::chainspec::OpChainSpecParser;

#[derive(Parser, Debug)]
#[command(version, about, name = "gateway")]
pub struct GatewayArgs {
    #[arg(
        long,
        value_name = "CHAIN_OR_PATH",
        long_help = OpChainSpecParser::help_message(),
        default_value = OpChainSpecParser::SUPPORTED_CHAINS[6],
        value_parser = OpChainSpecParser::parser(),
    )]
    pub chain_spec: Arc<OpChainSpec>,
    /// The host to run the engine_ and eth_ RPC
    #[arg(long = "rpc.host", default_value_t = Ipv4Addr::UNSPECIFIED)]
    pub rpc_host: Ipv4Addr,
    /// The port to run the engine_ and eth_ RPC
    #[arg(long = "rpc.port", default_value_t = 9090)]
    pub rpc_port: u16,
    /// Url to a full node for syncing and eth_ fallback requests
    #[arg(long = "rpc.fallback_url", default_value = "https://base-sepolia-rpc.publicnode.com")]
    pub rpc_fallback_url: Url,
    /// Duration of a frag in ms
    #[arg(long = "sequencer.frag_duration_ms", default_value_t = 200)]
    pub frag_duration_ms: u64,
    /// Number of sims per loop
    #[arg(long = "sequencer.sim_per_loop", default_value_t = 10)]
    pub sim_per_loop: usize,
    /// Coinbase address
    #[arg(long = "sequencer.coinbase")]
    pub coinbase: Address,
    /// Database location
    #[arg(long = "db.datadir")]
    pub db_datadir: PathBuf,
    /// Maximum number of cached accounts
    #[arg(long = "db.max_cached_accounts", default_value_t = 10_000)]
    pub max_cached_accounts: u64,
    /// Maximum number of cached storages
    #[arg(long = "db.max_cached_storages", default_value_t = 100_000)]
    pub max_cached_storages: u64,
    /// TMP END BLOCK
    #[arg(long = "tmp.end_block")]
    pub tmp_end_block: u64,
}
