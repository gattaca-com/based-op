use metrics_exporter_prometheus::PrometheusBuilder;

pub mod consumer;
pub use consumer::MetricsConsumer;

/// Installs the prometheus exporter on the given port.
pub fn install_prometheus_exporter(port: u16) {
    PrometheusBuilder::new()
        .with_http_listener(([0, 0, 0, 0], port))
        .add_global_label("instance", "based-optimism")
        .install()
        .expect("Failed to install prometheus exporter");
}
