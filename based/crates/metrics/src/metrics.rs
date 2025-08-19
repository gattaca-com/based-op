use bop_common::telemetry::system::SequencerState;
use metrics::{counter, gauge, histogram};

/// Contains all metrics for the based monitoring stack.
#[derive(Default)]
pub struct Metrics;

impl Metrics {
    pub fn increase_gateway_tx_added_to_pool_total() {
        counter!("bop_gateway_tx_added_to_pool_total").increment(1);
    }

    pub fn increase_gateway_tx_removed_from_pool_total() {
        counter!("bop_gateway_tx_removed_from_pool_total").increment(1);
    }

    pub fn increase_gateway_tx_included_total() {
        counter!("bop_gateway_tx_included_total").increment(1);
    }

    pub fn increase_gateway_tx_ingested_total() {
        counter!("bop_gateway_tx_ingested_total").increment(1);
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

    pub fn set_build_stop_block_number(block_number: u64) {
        gauge!("bop_build_stop_block_number").set(block_number as f64);
    }

    pub fn set_new_payload_block_number(block_number: u64) {
        gauge!("bop_new_payload_block_number").set(block_number as f64);
    }

    pub fn set_get_payload_block_number(block_number: u64) {
        gauge!("bop_get_payload_block_number").set(block_number as f64);
    }

    pub fn increase_sequencer_commit_frag_total() {
        counter!("bop_sequencer_commit_frag_total").increment(1);
    }

    pub fn record_frag_available_value(available_value: u64) {
        histogram!("bop_frag_available_value").record(available_value as f64);
    }

    pub fn increase_frag_sorter_start_total() {
        counter!("bop_frag_sorter_start_total").increment(1);
    }

    pub fn set_frag_current_block_number(block_number: u64) {
        gauge!("bop_frag_current_block_number").set(block_number as f64);
    }

    pub fn increase_frag_sorter_finish_total() {
        counter!("bop_frag_sorter_finish_total").increment(1);
    }

    pub fn record_frag_payment(payment: u64) {
        histogram!("bop_frag_payment").record(payment as f64);
    }

    pub fn record_frag_gas_used(gas_used: u64) {
        histogram!("bop_frag_gas_used").record(gas_used as f64);
    }

    pub fn record_frag_transaction_count(transaction_count: usize) {
        histogram!("bop_frag_transaction_count").record(transaction_count as f64);
    }

    pub fn record_frag_best_order_value(best_order_value: u64) {
        histogram!("bop_frag_best_order_value").record(best_order_value as f64);
    }
}
