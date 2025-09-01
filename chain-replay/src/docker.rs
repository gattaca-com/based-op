use std::{io, process::Command};

use crate::{
    chain::ChainName,
    utils::{get_ip, output_to_result},
};

/// Starts the based-op-geth service for the given chain, setting the sync target hash
pub fn start_based_op_service(chain_name: ChainName) -> io::Result<()> {
    let compose_file_path = chain_name.compose_file_path();
    let env_file_path = chain_name.env_file_path();

    let command_str = format!(
        "docker compose --file {} --env-file {} up -d based-op-geth",
        compose_file_path.to_string_lossy(),
        env_file_path.to_string_lossy()
    );
    let mut args = command_str.trim_matches('"').split(' ');

    let mut command = Command::new(args.next().expect("docker"));
    command.args(args);
    command.env("OP_GETH_EXTIP", get_ip().to_string());
    if chain_name.is_superchain() {
        command.env("OP_GETH_NETWORK", chain_name.to_string());
    }
    let output = command.output()?;

    output_to_result(&command_str, output)
}
