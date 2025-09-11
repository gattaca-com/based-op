//! Docker related utilities for starting services.

use std::{io, process::Command};

use crate::{
    chain::ChainName,
    config::Args,
    utils::{get_ip, output_to_result},
};

/// Docker compose down for all containers spun up via the chain compose file.
pub fn ensure_all_containers_down(chain_name: ChainName) -> io::Result<()> {
    let compose_file_path = chain_name.compose_file_path();
    let command_str = format!("docker compose --file {} down", compose_file_path.to_string_lossy());
    let mut args = command_str.trim_matches('"').split(' ');

    let mut command = Command::new(args.next().expect("docker"));
    command.args(args);
    let output = command.output()?;

    output_to_result(&command_str, output)
}

/// Starts the based-op-geth service for the given chain, setting the sync target hash
pub fn start_based_op_geth_service(chain_name: ChainName, instance_num: u8) -> io::Result<()> {
    let compose_file_path = chain_name.compose_file_path();
    let env_file_path = chain_name.env_file_path();

    let command_str = format!(
        "docker compose --file {} --env-file {} up -d bop-replay-based-op-geth-{}",
        compose_file_path.to_string_lossy(),
        env_file_path.to_string_lossy(),
        instance_num,
    );
    let mut args = command_str.trim_matches('"').split(' ');

    let mut command = Command::new(args.next().expect("docker"));
    command.args(args);
    let ip = get_ip().to_string();
    command.env(format!("BOP_REPLAY_BASED_OP_GETH_{instance_num}_EXTIP"), ip);
    if chain_name.is_superchain() {
        command.env(
            format!("BOP_REPLAY_BASED_OP_GETH_{instance_num}_NETWORK"),
            chain_name.to_string(),
        );
    } else if chain_name.is_based_op() {
        command.env(format!("BOP_REPLAY_BASED_OP_GETH_{instance_num}_BOOTNODES"), "enode://6439c0a0cad6b87b032d4e5bb401c5e34b91472eb33b6dbf6c09ed326dc9489d874ab0cacbd3e0f9f89dc7bcb32d66f09a040fb71505f68112037d48d782a509@18.185.199.51:30303");
    }
    let output = command.output()?;

    output_to_result(&command_str, output)
}

/// Start the based-registry service for the given chain.
pub fn start_based_registry(chain_name: ChainName) -> io::Result<()> {
    let compose_file_path = chain_name.compose_file_path();
    let env_file_path = chain_name.env_file_path();

    let command_str = format!(
        "docker compose --file {} --env-file {} up -d bop-replay-based-registry",
        compose_file_path.to_string_lossy(),
        env_file_path.to_string_lossy()
    );
    let mut args = command_str.trim_matches('"').split(' ');

    let mut command = Command::new(args.next().expect("docker"));
    command.args(args);
    let output = command.output()?;

    output_to_result(&command_str, output)
}

/// Starts the based-op-geth service for the given chain, setting the sync target hash
pub fn start_based_op_node_service(
    chain_name: ChainName,
    instance_num: u8,
    total_follower_peers: u8,
) -> io::Result<()> {
    let compose_file_path = chain_name.compose_file_path();
    let env_file_path = chain_name.env_file_path();

    let command_str = format!(
        "docker compose --file {} --env-file {} up -d bop-replay-based-op-node-{}",
        compose_file_path.to_string_lossy(),
        env_file_path.to_string_lossy(),
        instance_num,
    );
    let mut args = command_str.trim_matches('"').split(' ');

    let mut command = Command::new(args.next().expect("docker"));
    command.args(args);
    for i in 0..total_follower_peers {
        command.env(format!("BOP_REPLAY_BASED_OP_NODE_{i}_GOSSIP_IP"), get_ip().to_string());
    }
    command.env("BOP_REPLAY_KONA_SEQUENCER_IP", get_ip().to_string());
    let output = command.output()?;

    output_to_result(&command_str, output)
}

pub fn start_based_gatway(args: Args) -> io::Result<()> {
    let Args { chain_name, l2_el_verifier_url: l2_el_rpc_url, blocks_range, .. } = args;

    let compose_file_path = chain_name.compose_file_path();
    let env_file_path = chain_name.env_file_path();

    let command_str = format!(
        "docker compose --file {} --env-file {} up -d bop-replay-based-gateway",
        compose_file_path.to_string_lossy(),
        env_file_path.to_string_lossy()
    );
    let mut args = command_str.trim_matches('"').split(' ');

    let mut command = Command::new(args.next().expect("docker"));
    command.args(args);
    command.env("BOP_REPLAY_GATEWAY_REPLAY_TARGET", blocks_range.end().to_string());
    command.env("BOP_REPLAY_GATEWAY_REPLAY_L2_EL_VERIFIER_URL", l2_el_rpc_url.to_string());
    let output = command.output()?;

    output_to_result(&command_str, output)
}

pub fn start_monitoring_compose(chain_name: ChainName) -> io::Result<()> {
    let env_file_path = chain_name.env_file_path();

    let command_str = format!(
        "docker compose --file ./monitoring/compose.yml --env-file {} up -d",
        env_file_path.to_string_lossy()
    );
    let mut args = command_str.trim_matches('"').split(' ');

    let mut command = Command::new(args.next().expect("docker"));
    command.args(args);
    let output = command.output()?;

    output_to_result(&command_str, output)
}
