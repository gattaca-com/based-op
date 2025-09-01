use std::time::{Duration, Instant};

use bop_common::{
    communication::Consumer,
    metrics::{Gauge, Metric, MetricsUpdate, metrics_queue},
    telemetry::{Frag, Telemetry, TelemetryUpdate, Tx, system::SystemNotification, telemetry_queue},
};
use metrics::{counter, gauge, histogram};
use tracing::{info, trace};

/// The number of units of budget to spend per loop iteration.
/// Useful to avoid starving the consumers.
const LOOP_BUDGET: u64 = 500;

/// Consumes telemetry updates from shared memory queues, and converts them into metrics.
pub struct MetricsConsumer {
    telemetry: Consumer<TelemetryUpdate>,
    metrics: Consumer<MetricsUpdate>,

    budget: u64,
    event_count_checkpoint: Instant,
    event_count_since_checkpoint: u64,
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
                self.event_count_since_checkpoint += 1;
                if self.spend_budget() {
                    break;
                }
            }

            while let Some(update) = self.metrics.try_consume() {
                trace!(?update, "Received metrics update");
                self.process_metrics_queue_update(update);
                self.event_count_since_checkpoint += 1;
                if self.spend_budget() {
                    break;
                }
            }

            // Update the event count every second
            let elapsed = self.event_count_checkpoint.elapsed();
            if elapsed >= Duration::from_millis(1000) {
                let eps = self.event_count_since_checkpoint as f64 / elapsed.as_secs_f64();
                gauge!("bop_metric_events_per_second").set(eps);
                info!("Metric events/s: {:.2}", eps);

                self.event_count_checkpoint = Instant::now();
                self.event_count_since_checkpoint = 0;
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
                    gauge!("bop_frag_available_value").set(available_value.0 as f64);
                }
                Frag::SorterFinish { payment, best_order_value, n_txs, gas_used, .. } => {
                    counter!("bop_frags_sealed").increment(1);
                    gauge!("bop_frag_tx_count").set(n_txs as f64);
                    gauge!("bop_frag_payment").set(payment.0 as f64);
                    gauge!("bop_frag_gas_used").set(gas_used as f64);
                    gauge!("bop_frag_best_order_value").set(best_order_value.0 as f64);
                }
                Frag::Commit => counter!("bop_sequencer_commit_frag_total").increment(1),
            },
        }
    }

    /// Processes a metrics update, updating the corresponding metrics.
    fn process_metrics_queue_update(&mut self, update: MetricsUpdate) {
        // Note: we use strum's `AsRefStr` to get the metric name as a snake_case string.
        // For instance, `Counter::GatewayIngressTxsTotal.as_ref()` => "gateway_ingress_txs_total".
        match update.metric {
            Metric::IncrementCounter(counter, inc) => counter!(format!("bop_{}", counter.as_ref())).increment(inc),
            Metric::SetGauge(gauge, val) => {
                let name = format!("bop_{}", gauge.as_ref());
                match gauge {
                    Gauge::PortalGatewayPingLatencyMs(address) => {
                        gauge!(name, "address" => address.to_string()).set(val)
                    }
                    Gauge::PortalCurrentGatewayRegistryAddress(address) => {
                        gauge!(name, "address" => address.to_string()).set(val)
                    }
                    _ => gauge!(name).set(val),
                }
            }
            Metric::RecordHistogram(hist, val) => histogram!(format!("bop_{}", hist.as_ref())).record(val),
        }
    }
}

impl Default for MetricsConsumer {
    fn default() -> Self {
        Self {
            telemetry: telemetry_queue().into(),
            metrics: metrics_queue().into(),
            budget: LOOP_BUDGET,
            event_count_checkpoint: Instant::now(),
            event_count_since_checkpoint: 0,
        }
    }
}
