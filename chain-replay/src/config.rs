//! CLI arguments and configuration.

use std::{fmt::Debug, ops::RangeInclusive};

use alloy::{rpc::types::engine::JwtSecret, signers::local::PrivateKeySigner};
use url::Url;

use crate::chain::ChainName;

#[derive(clap::Parser, Debug, Clone)]
pub struct Args {
    /// The number of pairs of based-op-geth/based-op-node follower nodes. Defaults to 1.
    /// The maximum supported is currently 2.
    #[arg(
        long,
        env = "BOP_REPLAY_FOLLOWER_NODES_COUNT",
        id = "BOP_REPLAY_FOLLOWER_NODES_COUNT",
        default_value_t = 1
    )]
    pub follower_nodes_count: u8,
    /// The url of the gateway to send transactions to, non-authenticated.
    #[arg(long, env = "BOP_REPLAY_GATEWAY_URL", id = "BOP_REPLAY_GATEWAY_URL")]
    pub gateway_url: Url,
    /// The url of the gateway to engine API messages to, authenticated.
    #[arg(long, env = "BOP_REPLAY_GATEWAY_AUTH_URL", id = "BOP_REPLAY_GATEWAY_AUTH_URL")]
    pub gateway_auth_url: Url,
    #[arg(long, env = "BOP_REPLAY_GATEWAY_AUTH_JWT", id = "BOP_REPLAY_GATEWAY_AUTH_JWT")]
    pub gateway_auth_jwt: JwtSecret,
    /// The name of chain of which we want to replay blocks.
    #[clap(long, env = "BOP_REPLAY_CHAIN_NAME", id = "BOP_REPLAY_CHAIN_NAME")]
    pub chain_name: ChainName,
    /// The L2 execution layer RPC URL, needed to download the blocks to replay.
    #[clap(long, env = "BOP_REPLAY_L2_EL_VERIFIER_URL", id = "BOP_REPLAY_L2_EL_VERIFIER_URL")]
    pub l2_el_verifier_url: Url,
    #[arg(long, env = "BOP_REPLAY_P2P_SEQUENCER_KEY", id = "BOP_REPLAY_P2P_SEQUENCER_KEY")]
    pub p2p_sequencer_key: PrivateKeySigner,
    /// Port to listen for gossip on.
    #[arg(
        long,
        default_value = "9099",
        env = "BOP_REPLAY_KONA_SEQUENCER_GOSSIP_PORT",
        id = "BOP_REPLAY_KONA_SEQUENCER_GOSSIP_PORT"
    )]
    pub gossip_port: u16,
    /// Port to listen for discovery on.
    #[arg(
        long,
        default_value = "9098",
        env = "BOP_REPLAY_KONA_SEQUENCER_DISCOVERY_PORT",
        id = "BOP_REPLAY_KONA_SEQUENCER_DISCOVERY_PORT"
    )]
    pub disc_port: u16,
    /// The inclusive range of blocks to replay, in the format 'start..=end'.
    #[clap(
        long,
        env = "BOP_REPLAY_BLOCKS_RANGE",
        id = "BOP_REPLAY_BLOCKS_RANGE",
        value_parser = range_inclusive_from_str,
    )]
    pub blocks_range: RangeInclusive<u64>,
}

impl Args {
    pub fn validate(self) -> Self {
        if self.blocks_range.start() == &0 {
            panic!("Block range cannot start from genesis, must be at least block #1");
        }

        self
    }
}

fn range_inclusive_from_str(s: &str) -> Result<RangeInclusive<u64>, &'static str> {
    let parts = s.split("..=").collect::<Vec<_>>();
    if parts.len() != 2 {
        return Err("Range must be in the format 'start..=end'");
    }

    let Ok(start) = parts[0].parse::<u64>() else {
        return Err("Invalid start of range");
    };
    let Ok(end) = parts[1].parse::<u64>() else {
        return Err("Invalid end of range");
    };
    if start > end {
        return Err("Start of range must be less than or equal to end");
    }

    Ok(start..=end)
}
