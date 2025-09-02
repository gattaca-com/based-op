//! Chain-specific information and paths to where data is saved.

use std::{fmt::Display, path::PathBuf};

use clap::ValueEnum;

const LOCAL_CONFIG_PREFIX: &str = "../.local_gateway_and_follower";

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

    pub fn env_file_path(&self) -> PathBuf {
        self.directory_path().join(".env")
    }
}
