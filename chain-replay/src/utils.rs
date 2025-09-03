use std::{
    fs, io,
    net::Ipv4Addr,
    process::{self, Command},
    str::FromStr,
};

use alloy::rpc::types::engine::JwtSecret;

use crate::chain::ChainName;

pub fn ensure_chain_folder(chain_name: ChainName) -> io::Result<()> {
    let config_dir_path = chain_name.config_directory_path();

    if !fs::exists(&config_dir_path)? {
        println!("Creating config directory for chain '{chain_name}' at: {config_dir_path:?}");
        fs::create_dir_all(config_dir_path)?;
    }

    let jwt_path = chain_name.jwt_file_path();
    if !fs::exists(jwt_path.clone())? {
        println!(
            "Creating JWT file for chain '{chain_name}' at: {:?}",
            jwt_path.clone()
        );
        fs::copy("jwt", jwt_path)?;
    }

    let env_path = chain_name.env_file_path();
    if !fs::exists(env_path.clone())? {
        println!(
            "Creating .env file for chain '{chain_name}' at: {:?}",
            env_path.clone()
        );
        fs::copy(".env.example", env_path)?;
    }

    let compose_path = chain_name.compose_file_path();
    if !fs::exists(compose_path.clone())? {
        println!(
            "Creating docker-compose file for chain '{chain_name}' at: {:?}",
            compose_path.clone()
        );
        fs::copy("compose.yml", compose_path)?;
    }

    let genesis_path = chain_name.genesis_file_path();
    if !fs::exists(genesis_path.clone())? {
        println!(
            "Creating genesis.json file for chain '{chain_name}' at: {:?}",
            genesis_path.clone()
        );
        generate_genesis_file(chain_name)?;
    }

    let rollup_path = chain_name.rollup_file_path();
    if !fs::exists(rollup_path.clone())? {
        println!(
            "Creating rollup.json file for chain '{chain_name}' at: {:?}",
            rollup_path.clone()
        );
        generate_rollup_file(chain_name)?;
    }

    let registry_path = chain_name.registry_file_path();
    if !fs::exists(registry_path.clone())? {
        println!(
            "Creating registry.json file for chain '{chain_name}' at: {:?}",
            registry_path.clone()
        );
        fs::copy("registry.json", registry_path)?;
    }

    Ok(())
}

/// For the given chain, generate a valid genesis.json file using the based-op-geth image.
/// For chains part of OP-superchain, it consists of running the `dumpgenesis` command.
/// For based-op-sepolia, we'll copy the confguration inside the `sepolia-genesis` folder.
pub fn generate_genesis_file(chain_name: ChainName) -> io::Result<()> {
    match chain_name {
        ChainName::BasedOpSepolia => fs::copy(
            "../sepolia_genesis/genesis.json",
            chain_name.genesis_file_path(),
        )
        .map(|_| ()),
        chain => {
            let command_str = format!(
                "docker run ghcr.io/gattaca-com/based-op-geth/based-op-geth:latest --op-network {chain} dumpgenesis"
            );
            let mut args = command_str.split(' ');
            let mut command = Command::new(args.next().expect("docker"));
            let output = command.args(args).output()?;
            let stdout = extract_stdout(&command_str, output)?;
            fs::write(chain_name.genesis_file_path(), stdout)
        }
    }
}

/// For the given chain, generate a valid rollup.json file using the based-op-node image. For
/// chains part of OP-superchain, it consists of running the `networks dump-rollup-config` command.
/// For based-op-sepolia, we'll copy the confguration inside the `sepolia-genesis` folder.
pub fn generate_rollup_file(chain_name: ChainName) -> io::Result<()> {
    match chain_name {
        ChainName::BasedOpSepolia => fs::copy(
            "../sepolia_genesis/rollup.json",
            chain_name.rollup_file_path(),
        )
        .map(|_| ()),
        chain => {
            let command_str = format!(
                "docker run ghcr.io/gattaca-com/based-optimism/based-op-node:latest op-node networks dump-rollup-config --network {chain}"
            );
            let mut args = command_str.split(' ');
            let mut command = Command::new(args.next().expect("docker"));
            let output = command.args(args).output()?;
            let stdout = extract_stdout(&command_str, output)?;
            fs::write(chain_name.rollup_file_path(), stdout)
        }
    }
}

pub fn read_jwt_file(chain_name: ChainName) -> io::Result<JwtSecret> {
    let path = chain_name.jwt_file_path();

    let jwt_str = fs::read_to_string(path)?;

    let secret = JwtSecret::from_hex(&jwt_str).expect("to read valid jwt");

    Ok(secret)
}

/// Converts a process::Output into an io::Result, printing the stdout on success
pub fn output_to_result(command_str: &str, output: process::Output) -> io::Result<()> {
    if output.status.success() {
        let text = String::from_utf8_lossy(&output.stdout);
        println!("Command {command_str} executed successfully: {text}");
        Ok(())
    } else {
        let text = String::from_utf8_lossy(&output.stderr);
        Err(io::Error::other(format!(
            "Command {command_str} failed: {text}"
        )))
    }
}

/// Extracts stdout from a successfull output of a command, otherwise it returns an `io::Error`
/// capturing stderr.
pub fn extract_stdout(command_str: &str, output: process::Output) -> io::Result<Vec<u8>> {
    if output.status.success() {
        return Ok(output.stdout);
    }

    let text = String::from_utf8_lossy(&output.stderr);
    Err(io::Error::other(format!(
        "Command {command_str} failed: {text}"
    )))
}

/// Get the machine IP address by running `curl ifconfig.me`. Panics on failure.
pub fn get_ip() -> Ipv4Addr {
    let out = Command::new("curl")
        .arg("ifconfig.me")
        .output()
        .expect("to run curl ifconfig.me")
        .stdout;
    let text = String::from_utf8_lossy(&out);
    Ipv4Addr::from_str(&text).expect("valid ipv4 addr")
}
