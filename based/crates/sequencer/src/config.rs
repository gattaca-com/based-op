use bop_common::{config::GatewayArgs, time::Duration};
use reqwest::Url;
use reth_optimism_chainspec::{BASE_MAINNET, BASE_SEPOLIA};
use reth_optimism_evm::OpEvmConfig;

use crate::block_sync::fetch_blocks::{TEST_BASE_RPC_URL, TEST_BASE_SEPOLIA_RPC_URL};

#[derive(Clone, Debug)]
pub struct SequencerConfig {
    pub frag_duration: Duration,
    pub n_per_loop: usize,
    pub rpc_url: Url,
    pub evm_config: OpEvmConfig,
    /// If true, we will simulate txs at the top of each frag in the pools.
    pub simulate_tof_in_pools: bool,
}

impl SequencerConfig {
    pub fn default_base_mainnet() -> Self {
        let chainspec = BASE_MAINNET.clone();
        let evm_config = OpEvmConfig::new(chainspec);

        Self {
            frag_duration: Duration::from_millis(200),
            n_per_loop: 200,
            rpc_url: Url::parse(TEST_BASE_RPC_URL).unwrap(),
            simulate_tof_in_pools: false,
            evm_config,
        }
    }

    pub fn default_base_sepolia() -> Self {
        let chainspec = BASE_SEPOLIA.clone();
        let evm_config = OpEvmConfig::new(chainspec);

        Self {
            frag_duration: Duration::from_millis(200),
            n_per_loop: 200,
            rpc_url: Url::parse(TEST_BASE_SEPOLIA_RPC_URL).unwrap(),
            simulate_tof_in_pools: false,
            evm_config,
        }
    }
}

impl From<&GatewayArgs> for SequencerConfig {
    fn from(args: &GatewayArgs) -> Self {
        Self {
            frag_duration: Duration::from_millis(args.frag_duration_ms),
            n_per_loop: args.sim_per_loop,
            rpc_url: args.rpc_fallback_url.clone(),
            simulate_tof_in_pools: false,
            evm_config: OpEvmConfig::new(args.chain_spec.clone()),
        }
    }
}
