use bop_common::{
    communication::Consumer,
    metrics::{Gauge, Metric, MetricsUpdate, metrics_queue},
    telemetry::{Frag, Telemetry, TelemetryUpdate, Tx, system::SystemNotification, telemetry_queue},
    time::{Duration, Instant, Repeater},
};
use metrics::{counter, gauge, histogram};
use tracing::{info, trace};

/// Consumes telemetry updates from shared memory queues, and converts them into metrics.
pub struct MetricsConsumer {
    telemetry: Consumer<TelemetryUpdate>,
    metrics: Consumer<MetricsUpdate>,
}

impl MetricsConsumer {
    /// Runs the metrics consumer, consuming telemetry updates from shared queues,
    /// and converting them into metrics.
    pub fn run(mut self) {
        let tick_dur = Duration::from_millis(1);

        let mut tick = Repeater::every(Duration::from_millis(1000));
        let mut event_count_last_checkpoint = Instant::now();
        let mut event_count_since_checkpoint = 0;

        loop {
            let start = Instant::now();
            consume_for(&mut self.telemetry, tick_dur, |update| {
                trace!(?update, "Received telemetry update");
                Self::process_telemetry_queue_update(update);
                event_count_since_checkpoint += 1;
            });

            consume_for(&mut self.metrics, tick_dur, |update| {
                trace!(?update, "Received metrics update");
                Self::process_metrics_queue_update(update);
                event_count_since_checkpoint += 1;
            });

            if tick.fired() {
                let elapsed = event_count_last_checkpoint.elapsed();
                let eps = event_count_since_checkpoint as f64 / elapsed.as_secs();
                gauge!("bop_metric_events_per_second").set(eps);
                info!("Metric events/s: {:.2}", eps);

                event_count_last_checkpoint = Instant::now();
                event_count_since_checkpoint = 0;
            };

            let rem = tick_dur.saturating_sub(start.elapsed());
            if rem > Duration::from_nanos(0) {
                std::thread::sleep(rem.into());
            }
        }
    }

    /// Processes a telemetry update, converting it into metrics.
    fn process_telemetry_queue_update(update: TelemetryUpdate) {
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
    fn process_metrics_queue_update(update: MetricsUpdate) {
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
        Self { telemetry: telemetry_queue().into(), metrics: metrics_queue().into() }
    }
}

/// Consumes items from a consumer for a specified duration, calling the provided function on each item.
/// The function is called immediately for each consumed item.
fn consume_for<T: Clone, F>(consumer: &mut Consumer<T>, duration: Duration, mut f: F)
where
    F: FnMut(T),
{
    let start = Instant::now();
    while let Some(item) = consumer.try_consume() {
        f(item);

        if start.elapsed() > duration {
            break;
        }
    }
}
