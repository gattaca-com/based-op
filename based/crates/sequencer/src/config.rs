use alloy_primitives::Address;
use bop_common::time::Duration;
use reqwest::Url;
use reth_optimism_chainspec::{BASE_MAINNET, BASE_SEPOLIA};
use reth_optimism_evm::OpEvmConfig;

use crate::block_sync::fetch_blocks::{TEST_BASE_RPC_URL, TEST_BASE_SEPOLIA_RPC_URL};

#[derive(Clone, Debug)]
pub struct SequencerConfig {
    pub frag_duration: Duration,
    pub max_gas: u64,
    pub n_per_loop: usize,
    pub rpc_url: Url,
    pub evm_config: OpEvmConfig,
    pub coinbase: Address,
}

impl SequencerConfig {
    pub fn default_base_mainnet() -> Self {
        let chainspec = BASE_MAINNET.clone();
        let evm_config = OpEvmConfig::new(chainspec);

        Self {
            frag_duration: Duration::from_millis(200),
            max_gas: 300_000_000,
            n_per_loop: 10,
            rpc_url: Url::parse(TEST_BASE_RPC_URL).unwrap(),
            evm_config,
            coinbase: Address::random(),
        }
    }

    pub fn default_base_sepolia() -> Self {
        let chainspec = BASE_SEPOLIA.clone();
        let evm_config = OpEvmConfig::new(chainspec);

        Self {
            frag_duration: Duration::from_millis(200),
            max_gas: 300_000_000,
            n_per_loop: 10,
            rpc_url: Url::parse(TEST_BASE_SEPOLIA_RPC_URL).unwrap(),
            evm_config,
            coinbase: Address::random(),
        }
    }
}
