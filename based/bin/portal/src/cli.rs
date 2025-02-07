use std::{net::Ipv4Addr, path::PathBuf};

use clap::{command, Parser};
use eyre::bail;
use reqwest::Url;
use reth_rpc_layer::JwtSecret;

#[derive(Parser, Debug, Clone)]
#[command(version, about, name = "based-portal")]
pub struct PortalArgs {
    /// The host to run the portal on
    #[arg(long = "portal.host", default_value_t = Ipv4Addr::UNSPECIFIED)]
    pub portal_host: Ipv4Addr,

    /// The port to run the portal on
    #[arg(long = "portal.port", default_value_t = 8080)]
    pub portal_port: u16,

    /// The URL to the fallback EngineAPI
    #[arg(long = "fallback.url")]
    pub fallback_url: Url,

    /// Timeout for fallback requests in milliseconds
    #[arg(long = "fallback.timeout_ms", default_value_t = 1_000)]
    pub fallback_timeout_ms: u64,

    /// The JWT token to use for the fallback
    #[arg(long = "fallback.jwt", conflicts_with = "fallback_jwt_path")]
    pub fallback_jwt: Option<JwtSecret>,

    /// Path to the JWT token file to use for the fallback
    #[arg(long = "fallback.jwt_path", conflicts_with = "fallback_jwt")]
    pub fallback_jwt_path: Option<PathBuf>,

    /// The URL to the gateway EngineAPI
    #[arg(long = "gateway.url")]
    pub gateway_url: Url,

    /// Timeout for gateway requests in milliseconds
    #[arg(long = "gateway.timeout_ms", default_value_t = 1_000)]
    pub gateway_timeout_ms: u64,

    /// The JWT token to use for the fallback
    #[arg(long = "gateway.jwt", conflicts_with = "gateway_jwt_path")]
    pub gateway_jwt: Option<JwtSecret>,

    /// Path to the JWT token file to use for the gateway
    #[arg(long = "gateway.jwt_path", conflicts_with = "gateway_jwt")]
    pub gateway_jwt_path: Option<PathBuf>,

    /// Enable debug logging
    #[arg(long)]
    pub debug: bool,

    /// Enable trace logging
    #[arg(long)]
    pub trace: bool,
}

impl PortalArgs {
    pub fn fallback_jwt(&self) -> eyre::Result<JwtSecret> {
        if let Some(jwt) = self.fallback_jwt {
            Ok(jwt)
        } else if let Some(path) = self.fallback_jwt_path.as_ref() {
            let jwt = JwtSecret::from_file(path)?;
            Ok(jwt)
        } else {
            bail!("either --fallback.jwt or --fallback.jwt_path must be provided");
        }
    }

    pub fn gateway_jwt(&self) -> eyre::Result<JwtSecret> {
        if let Some(jwt) = self.gateway_jwt {
            Ok(jwt)
        } else if let Some(path) = self.gateway_jwt_path.as_ref() {
            let jwt = JwtSecret::from_file(path)?;
            Ok(jwt)
        } else {
            bail!("either --gateway.jwt or --gateway.jwt_path must be provided");
        }
    }
}
