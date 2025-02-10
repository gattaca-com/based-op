use bop_common::{config::GatewayArgs, time::Duration};
use reqwest::Url;
use reth_optimism_evm::OpEvmConfig;

#[derive(Clone, Debug)]
pub struct SequencerConfig {
    pub frag_duration: Duration,
    pub n_per_loop: usize,
    pub rpc_url: Url,
    pub evm_config: OpEvmConfig,
    /// If true, we will simulate txs at the top of each frag in the pools.
    pub simulate_tof_in_pools: bool,
}

impl From<&GatewayArgs> for SequencerConfig {
    fn from(args: &GatewayArgs) -> Self {
        Self {
            frag_duration: Duration::from_millis(args.frag_duration_ms),
            n_per_loop: args.sim_threads,
            rpc_url: args.rpc_fallback_url.clone(),
            simulate_tof_in_pools: false,
            evm_config: OpEvmConfig::new(args.chain_spec.clone()),
        }
    }
}
