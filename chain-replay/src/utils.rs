use std::{fs, io};

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

    // TODO: add genesis.json and rollup.json

    Ok(())
}
