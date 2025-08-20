use bop_metrics::{consumer::MetricsConsumer, install_prometheus_exporter};
use clap::Parser;

/// Exposes metrics for Prometheus to scrape.
#[derive(Debug, Parser)]
struct Args {
    /// The port to expose the metrics on. Default is 9464.
    #[clap(short, long, default_value = "9464")]
    port: u16,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    install_prometheus_exporter(args.port);

    MetricsConsumer::default().run().await;
}
