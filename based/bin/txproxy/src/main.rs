use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use bop_common::{metrics::install_prometheus_exporter, utils::init_tracing};
use bop_metrics::MetricsConsumer;
use clap::Parser;
use cli::TxProxyArgs;
use server::TxProxyServer;
use tracing::info;
mod cli;
mod middleware;
mod server;

#[tokio::main]
async fn main() -> eyre::Result<()> {
    let args = TxProxyArgs::parse();
    let _guard = init_tracing((&args).into());

    let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), args.txproxy_port);
    let server = TxProxyServer::new(args.clone()).await?;

    if args.enable_metrics {
        install_prometheus_exporter(args.metrics_port);
        std::thread::spawn(move || {
            let consumer = MetricsConsumer::default();
            info!("Prometheus server started on port {}", args.metrics_port);
            consumer.run();
        });
    }

    info!(%addr, "starting TxProxy server");
    server.run(addr).await
}
