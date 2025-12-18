mod api;
mod cli;
pub mod driver;
mod error;
pub mod exec;
pub mod unsealed_block;

pub use cli::*;

#[cfg(test)]
mod tests {
    #[test]
    fn test_cli() {
        crate::run_from_args([
            "op-reth",
            "node",
            "--chain",
            "base-sepolia",
            "--datadir",
            "/tmp/base-sepolia",
            "--http",
            "--http.addr",
            "0.0.0.0",
            "--ws",
            "--ws.addr",
            "0.0.0.0",
            "--http.api",
            "admin,debug,eth,net,trace,txpool,web3,rpc,reth,ots",
            "--rollup.sequencer-http",
            "https://sepolia-sequencer.base.org",
            "--rpc-max-tracing-requests",
            "1000000",
            "--rpc.gascap",
            "18446744073709551615",
            "--rpc.max-connections",
            "429496729",
            "--rpc.max-logs-per-response",
            "0",
            "--rpc.max-subscriptions-per-connection",
            "10000",
            "--metrics",
            "9003",
            "--unsealed-as-latest",
        ])
        .unwrap();
    }
}
