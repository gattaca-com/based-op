use bop_common::{config::GatewayArgs, time::Duration};
use kona_interop::SafetyLevel;
use reqwest::Url;
use reth_optimism_evm::OpEvmConfig;
use reth_optimism_payload_builder::config::OpDAConfig;

#[derive(Clone, Debug)]
pub struct SuperVisorConfig {
    pub url: Url,
    pub safety_level: SafetyLevel,
}

impl From<&GatewayArgs> for SuperVisorConfig {
    fn from(value: &GatewayArgs) -> Self {
        let (Some(url), Some(safety)) = (value.supervisor_url.clone(), &value.supervisor_safety_level) else {
            panic!(
                "Trying to create supervisor without setting the right parameters (need supervisor.url and supervisor.safety_level)"
            );
        };
        let safety_level = serde_json::from_str(safety).expect("couldn't parse safety");
        Self { url, safety_level }
    }
}

#[derive(Clone, Debug)]
pub struct SequencerConfig {
    pub frag_duration: Duration,
    pub n_per_loop: usize,
    pub fifo_ordering: bool,
    pub rpc_url: Url,
    pub evm_config: OpEvmConfig,
    /// If true, we will simulate txs at the top of each frag in the pools.
    pub simulate_tof_in_pools: bool,
    /// If true will commit locally sequenced blocks to the db before getting payload from the engine api.
    pub commit_sealed_frags_to_db: bool,
    pub supervisor: Option<SuperVisorConfig>,
    pub da_config: OpDAConfig,
}

impl From<&GatewayArgs> for SequencerConfig {
    fn from(args: &GatewayArgs) -> Self {
        Self {
            frag_duration: Duration::from_millis(args.frag_duration_ms),
            n_per_loop: if args.fifo_ordering { 1 } else { args.sim_threads },
            fifo_ordering: args.fifo_ordering,
            rpc_url: args.eth_client_url.clone(),
            simulate_tof_in_pools: false,
            evm_config: OpEvmConfig::new(args.chain.clone(), Default::default()),
            commit_sealed_frags_to_db: args.commit_sealed_frags_to_db,
            supervisor: args.supervisor_url.as_ref().map(|_| SuperVisorConfig::from(args)),
            da_config: args.da_config.clone(),
        }
    }
}
