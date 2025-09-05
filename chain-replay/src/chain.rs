//! Chain-specific information and paths to where data is saved.

use std::{fmt::Display, fs, io, path::PathBuf};

use clap::ValueEnum;

use crate::utils::{generate_genesis_file, generate_rollup_file};

const LOCAL_CONFIG_PREFIX: &str = "../../.local_gateway_and_follower";

#[derive(Debug, Clone, Copy, ValueEnum)]
#[clap(rename_all = "kebab-case")]
pub enum ChainName {
    BaseMainnet,
    BaseSepolia,
    BasedOpSepolia,
}

impl Display for ChainName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            ChainName::BaseMainnet => "base-mainnet",
            ChainName::BaseSepolia => "base-sepolia",
            ChainName::BasedOpSepolia => "based-op-sepolia",
        };
        write!(f, "{s}")
    }
}

impl ChainName {
    pub fn is_superchain(&self) -> bool {
        (matches!(&self, Self::BaseSepolia) || matches!(&self, Self::BaseMainnet))
    }
    pub fn is_based_op(&self) -> bool {
        matches!(&self, Self::BasedOpSepolia)
    }
}

#[allow(dead_code)]
impl ChainName {
    fn directory_path(&self) -> PathBuf {
        PathBuf::from(format!("{LOCAL_CONFIG_PREFIX}_{self}"))
    }

    pub fn config_directory_path(&self) -> PathBuf {
        self.directory_path().join("config")
    }

    pub fn jwt_file_path(&self) -> PathBuf {
        self.config_directory_path().join("jwt")
    }

    pub fn compose_file_path(&self) -> PathBuf {
        self.directory_path().join("compose.yml")
    }

    pub fn genesis_file_path(&self) -> PathBuf {
        self.config_directory_path().join("genesis.json")
    }

    pub fn rollup_file_path(&self) -> PathBuf {
        self.config_directory_path().join("rollup.json")
    }

    pub fn registry_file_path(&self) -> PathBuf {
        self.config_directory_path().join("registry.json")
    }

    pub fn env_file_path(&self) -> PathBuf {
        self.directory_path().join(".env")
    }
}

pub fn ensure_chain_folder(chain_name: ChainName) -> io::Result<()> {
    let config_dir_path = chain_name.config_directory_path();

    if !fs::exists(&config_dir_path)? {
        println!("Creating config directory for chain '{chain_name}' at: {config_dir_path:?}");
        fs::create_dir_all(config_dir_path)?;
    }

    let jwt_path = chain_name.jwt_file_path();
    if !fs::exists(jwt_path.clone())? {
        println!("Creating JWT file for chain '{chain_name}' at: {:?}", jwt_path.clone());
        fs::copy("jwt", jwt_path)?;
    }

    let env_path = chain_name.env_file_path();
    if !fs::exists(env_path.clone())? {
        println!("Creating .env file for chain '{chain_name}' at: {:?}", env_path.clone());
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
