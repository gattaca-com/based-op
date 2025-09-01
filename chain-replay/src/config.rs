use std::{fmt::Debug, ops::RangeInclusive};

use url::Url;

use crate::chain::ChainName;

#[derive(clap::Parser, Debug)]
pub struct Args {
    /// The name of chain of which we want to replay blocks.
    #[clap(long, env = "BASED_OP_CHAIN_NAME")]
    pub chain_name: ChainName,
    /// The L2 engine RPC URL, needed to send CL messages.
    #[clap(long, env = "BASED_OP_L2_ENGINE_RPC_URL")]
    pub l2_engine_rpc_url: Url,
    /// The L2 execution layer RPC URL, needed to download the blocks to replay.
    #[clap(long, env = "BASED_OP_L2_EL_RPC_URL")]
    pub l2_el_rpc_url: Url,
    /// An L2 execution layer bootnode, needed to connect to the network. It is optional for
    /// chains part of OP superchain.
    #[clap(long, env = "BASED_OP_L2_EL_BOOTNODE")]
    pub l2_el_bootnode: Option<String>,
    /// The inclusive range of blocks to replay, in the format 'start..=end'.
    #[clap(long, env = "BASED_OP_BLOCKS_RANGE", value_parser = range_inclusive_from_str)]
    pub blocks_range: RangeInclusive<u64>,
}

impl Args {
    pub fn validate(self) -> Self {
        if self.blocks_range.start() == &0 {
            panic!("Block range cannot start from genesis, must be at least block #1");
        }

        if self.l2_el_bootnode.is_none() && matches!(self.chain_name, ChainName::BasedOpSepolia) {
            panic!("L2 EL bootnode is required for based-op-sepolia");
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
