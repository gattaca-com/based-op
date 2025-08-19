/// Contains all metrics for the based monitoring stack.
#[derive(Default)]
pub struct Metrics;

impl Metrics {
    pub fn increase_gateway_tx_added_to_pool_total() {
        metrics::counter!("bop_gateway_tx_added_to_pool_total").increment(1);
    }

    pub fn increase_gateway_tx_included_total() {
        metrics::counter!("bop_gateway_tx_included_total").increment(1);
    }
}
