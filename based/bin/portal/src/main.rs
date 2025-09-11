use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use bop_common::{metrics::install_prometheus_exporter, utils::init_tracing};
use bop_metrics::MetricsConsumer;
use clap::Parser;
use cli::PortalArgs;
use server::PortalServer;
use tracing::info;

mod cli;
mod clients;
mod middleware;
mod server;

#[tokio::main]
async fn main() -> eyre::Result<()> {
    let args = PortalArgs::parse();
    let _guard = init_tracing((&args).into());

    let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), args.portal_port);
    let server = PortalServer::new(args.clone()).await?;

    if args.enable_metrics {
        install_prometheus_exporter(args.metrics_port);
        std::thread::spawn(move || {
            let consumer = MetricsConsumer::default();
            info!("Prometheus server started on port {}", args.metrics_port);
            consumer.run();
        });
    }

    info!(%addr, registry_url = %args.registry_url, fallback_url = %args.fallback_url, fallback_eth_url = %args.fallback_eth_url, "starting Based Portal");

    server.run(addr).await
}
