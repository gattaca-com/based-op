use std::time::Duration;

use bop_common::{
    communication::Consumer,
    metrics::{Metric, MetricsUpdate, metrics_queue},
    telemetry::{Frag, Telemetry, TelemetryUpdate, Tx, system::SystemNotification, telemetry_queue},
};
use metrics::{counter, gauge, histogram};
use tracing::trace;

/// The number of units of budget to spend per loop iteration.
/// Useful to avoid starving the consumers.
const LOOP_BUDGET: u64 = 10_000;

/// Consumes telemetry updates from shared memory queues, and converts them into metrics.
pub struct MetricsConsumer {
    telemetry: Consumer<TelemetryUpdate>,
    metrics: Consumer<MetricsUpdate>,
    budget: u64,
}

impl MetricsConsumer {
    /// Spends one unit of budget, and returns true if the budget is exhausted.
    /// If the budget is exhausted, it is reset for the next iteration.
    fn spend_budget(&mut self) -> bool {
        self.budget -= 1;

        let exhausted = self.budget == 0;
        if exhausted {
            self.budget = LOOP_BUDGET;
        }

        exhausted
    }

    /// Runs the metrics consumer, consuming telemetry updates from shared queues,
    /// and converting them into metrics.
    pub async fn run(mut self) {
        loop {
            while let Some(update) = self.telemetry.try_consume() {
                trace!(?update, "Received telemetry update");
                self.process_telemetry_queue_update(update);
                if self.spend_budget() {
                    break;
                }
            }

            while let Some(update) = self.metrics.try_consume() {
                trace!(?update, "Received metrics update");
                self.process_metrics_queue_update(update);
                if self.spend_budget() {
                    break;
                }
            }

            tokio::time::sleep(Duration::from_millis(20)).await
        }
    }

    /// Processes a telemetry update, converting it into metrics.
    fn process_telemetry_queue_update(&mut self, update: TelemetryUpdate) {
        match update.update {
            Telemetry::Tx(tx) => match tx {
                Tx::AddedToPool => counter!("bop_gateway_tx_added_to_pool_total").increment(1),
                Tx::RemovedFromPool => counter!("bop_gateway_tx_removed_from_pool_total").increment(1),
                Tx::Included(_) => counter!("bop_gateway_tx_included_total").increment(1),
                Tx::Ingested(_) => counter!("bop_gateway_tx_ingested_total").increment(1),
            },
            Telemetry::System(system) => match system {
                SystemNotification::StateChanged(state) => gauge!("bop_sequencer_state").set(state as u8),
                SystemNotification::BlockSync(block_num, _) => {
                    gauge!("bop_block_sync_block_number").set(block_num as f64)
                }
                SystemNotification::Sorting(block_num) => gauge!("bop_sorting_block_number").set(block_num as f64),
                SystemNotification::BuildStop(block_num) => gauge!("bop_build_stop_block_number").set(block_num as f64),
                SystemNotification::NewPayload(block_num) => {
                    gauge!("bop_new_payload_block_number").set(block_num as f64)
                }
                SystemNotification::GetPayload(block_num) => {
                    gauge!("bop_get_payload_block_number").set(block_num as f64)
                }
                SystemNotification::ForkChoiceUpdate(_block_hash) => { /* event skipped*/ }
            },
            Telemetry::Frag(frag) => match frag {
                Frag::SorterStart { block, available_value, .. } => {
                    counter!("bop_frag_sorter_start_total").increment(1);
                    gauge!("bop_frag_current_block_number").set(block as f64);
                    histogram!("bop_frag_available_value").record(available_value.0 as f64);
                }
                Frag::SorterFinish { payment, best_order_value, n_txs, gas_used, .. } => {
                    counter!("bop_frag_sorter_finish_total").increment(1);
                    histogram!("bop_frag_payment").record(payment.0 as f64);
                    histogram!("bop_frag_gas_used").record(gas_used as f64);
                    histogram!("bop_frag_transaction_count").record(n_txs as f64);
                    histogram!("bop_frag_best_order_value").record(best_order_value.0 as f64);
                }
                Frag::Commit => counter!("bop_sequencer_commit_frag_total").increment(1),
            },
        }
    }

    /// Processes a metrics update, updating the corresponding metrics.
    fn process_metrics_queue_update(&mut self, update: MetricsUpdate) {
        // Note: we use strum's `AsRefStr` to get the metric name as a snake_case string.
        // For instance, `Counter::GatewayIngressTxsTotal.as_ref()` => "gateway_ingress_txs_total".
        //
        // Values are extracted using the `value()` method for the enum.
        match update.metric {
            Metric::IncrementCounter(counter) => counter!(format!("bop_{}", counter.as_ref())).increment(1),
            Metric::SetGauge(gauge) => gauge!(format!("bop_{}", gauge.as_ref())).set(gauge.value()),
            Metric::RecordHistogram(hist) => histogram!(format!("bop_{}", hist.as_ref())).record(hist.value()),
        }
    }
}

impl Default for MetricsConsumer {
    fn default() -> Self {
        Self { telemetry: telemetry_queue().into(), metrics: metrics_queue().into(), budget: LOOP_BUDGET }
    }
}
