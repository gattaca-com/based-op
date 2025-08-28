use std::{
    io,
    process::{self, Command},
};

use alloy::primitives::B256;

use crate::chain::ChainName;

/// Creates a basic `docker compose` command to spin a up a service for the given chain.
/// Returns a command just without the service name and any additional environment variables.
pub fn basic_compose_command(chain_name: ChainName) -> Command {
    let compose_file_path = chain_name.compose_file_path();
    let env_file_path = chain_name.env_file_path();

    let mut command = Command::new("docker");
    command
        .arg("compose")
        .arg("--file")
        .arg(compose_file_path)
        .arg("--env-file")
        .arg(env_file_path)
        .arg("up")
        .arg("-d");
    command
}

pub fn start_based_op_service(chain_name: ChainName, sync_target_hash: B256) -> io::Result<()> {
    let command = basic_compose_command(chain_name)
        .arg("based-op-geth")
        .env("OP_GETH_SYNC_TARGET", format!("{sync_target_hash:?}"))
        .output()?;

    output_to_result(command, "based-op-geth")?;

    Ok(())
}

fn output_to_result(output: process::Output, service: &str) -> io::Result<()> {
    if output.status.success() {
        let text = String::from_utf8_lossy(&output.stdout);
        println!("Service '{service}' started successfully: {text}");
    } else {
        let text = String::from_utf8_lossy(&output.stderr);
        return Err(io::Error::other(format!(
            "Failed to start service '{service}': {}",
            text
        )));
    }
    Ok(())
}
