use bop_common::telemetry::system::SequencerState;
use metrics::{counter, gauge};

/// Contains all metrics for the based monitoring stack.
#[derive(Default)]
pub struct Metrics;

impl Metrics {
    pub fn increase_gateway_tx_added_to_pool_total() {
        counter!("bop_gateway_tx_added_to_pool_total").increment(1);
    }

    pub fn increase_gateway_tx_included_total() {
        counter!("bop_gateway_tx_included_total").increment(1);
    }

    pub fn set_sequencer_state(state: SequencerState) {
        gauge!("bop_sequencer_state").set(state as u8);
    }

    pub fn set_block_sync_block_number(block_number: u64) {
        gauge!("bop_block_sync_block_number").set(block_number as f64);
    }

    pub fn set_sorting_block_number(block_number: u64) {
        gauge!("bop_sorting_block_number").set(block_number as f64);
    }
}
