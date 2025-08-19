use bop_common::utils::wait_for_signal;
use bop_metrics::{MetricsConsumer, install_prometheus_exporter};
use clap::Parser;
use tracing::{info, level_filters::LevelFilter};
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

/// Exposes metrics for Prometheus to scrape.
#[derive(Debug, Parser)]
struct Args {
    /// The port to expose the metrics on. Default is 9464.
    #[clap(short, long, default_value = "9464")]
    port: u16,
}

#[tokio::main]
async fn main() {
    // Parse CLI arguments
    let args = Args::parse();

    // Init tracing
    let env = EnvFilter::builder().with_default_directive(LevelFilter::INFO.into()).from_env_lossy();
    tracing_subscriber::registry().with(env).with(tracing_subscriber::fmt::layer()).init();

    // Init Prometheus server
    install_prometheus_exporter(args.port);
    info!("Prometheus server started on port {}", args.port);

    let consumer = MetricsConsumer::default();

    // Start the metrics consumer loop
    tokio::select! {
        _ = consumer.run() => {},
        _ = wait_for_signal() => info!("Shutdown signal received")
    }
}
