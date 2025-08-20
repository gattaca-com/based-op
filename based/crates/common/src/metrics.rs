use serde::{Deserialize, Serialize};
use strum_macros::AsRefStr;
use uuid::Uuid;

use crate::{
    communication::{Producer, Queue, queue::QueueType, queues_dir},
    time::Nanos,
};

/// Creates or opens the metrics queue.
pub fn metrics_queue() -> Queue<MetricsUpdate> {
    Queue::create_or_open_shared(queues_dir().join("metrics"), 2usize.pow(18), QueueType::MPMC)
        .expect("Can't create or open metrics queue")
}

/// Lightweight in-memory metrics aggregator to avoid queue flooding
/// Collects metrics during operations and flushes aggregated values periodically
#[derive(Default, Clone, Debug)]
pub struct MetricsAggregator {
    // Sim metrics
    sim_requests: u64,
    sim_results: u64,
    sim_errors: u64,
    sim_latencies: Vec<f64>, // Keep last 100 samples for histogram

    // Frag metrics
    frags_created: u64,
    frags_sealed: u64,
    frag_tx_counts: Vec<u64>, // Keep last 100 samples for histogram
    frag_durations: Vec<f64>, // Keep last 100 samples for histogram

    // Block metrics
    blocks_built: u64,
    block_durations: Vec<f64>, // Keep last 100 samples for histogram

    // Pool metrics
    active_tx_count: usize,
    pool_memory_bytes: usize,

    // Block height
    current_block_height: u64,

    // Sim queue state
    simulation_queue_depth: usize,
    simulation_in_flight_count: usize,
}

impl MetricsAggregator {
    // Simulation metrics
    pub fn inc_simulation_request(&mut self) {
        self.sim_requests += 1;
    }

    pub fn inc_simulation_result(&mut self) {
        self.sim_results += 1;
    }

    pub fn inc_simulation_error(&mut self) {
        self.sim_errors += 1;
    }

    pub fn record_simulation_latency(&mut self, latency: f64) {
        self.sim_latencies.push(latency);
        if self.sim_latencies.len() > 100 {
            self.sim_latencies.remove(0);
        }
    }

    // Fragment metrics
    pub fn inc_fragment_created(&mut self) {
        self.frags_created += 1;
    }

    pub fn inc_fragment_sealed(&mut self) {
        self.frags_sealed += 1;
    }

    pub fn record_fragment_tx_count(&mut self, tx_count: u64) {
        self.frag_tx_counts.push(tx_count);
        if self.frag_tx_counts.len() > 100 {
            self.frag_tx_counts.remove(0);
        }
    }

    pub fn record_fragment_duration(&mut self, duration: f64) {
        self.frag_durations.push(duration);
        if self.frag_durations.len() > 100 {
            self.frag_durations.remove(0);
        }
    }

    // Block metrics
    pub fn inc_block_built(&mut self) {
        self.blocks_built += 1;
    }

    pub fn record_block_duration(&mut self, duration: f64) {
        self.block_durations.push(duration);
        if self.block_durations.len() > 100 {
            self.block_durations.remove(0);
        }
    }

    // Pool metrics
    pub fn set_active_tx_count(&mut self, count: usize) {
        self.active_tx_count = count;
    }

    pub fn set_pool_memory_bytes(&mut self, bytes: usize) {
        self.pool_memory_bytes = bytes;
    }

    // Block height
    pub fn set_block_height(&mut self, height: u64) {
        self.current_block_height = height;
    }

    // Simulation queue state
    pub fn set_simulation_queue_depth(&mut self, depth: usize) {
        self.simulation_queue_depth = depth;
    }

    pub fn set_simulation_in_flight_count(&mut self, count: usize) {
        self.simulation_in_flight_count = count;
    }

    /// Flush aggregated metrics to the queue and reset counters
    pub fn flush_to_queue(&mut self, producer: &mut Producer<MetricsUpdate>) {
        let uuid = Uuid::new_v4();

        // Send aggregated counters
        MetricsUpdate::send(
            uuid,
            Metric::IncrementCounter(Counter::SimulationRequestsSent, self.sim_requests),
            producer,
        );

        MetricsUpdate::send(
            uuid,
            Metric::IncrementCounter(Counter::SimulationResultsReceived, self.sim_results),
            producer,
        );
        MetricsUpdate::send(uuid, Metric::IncrementCounter(Counter::SimulationErrors, self.sim_errors), producer);
        MetricsUpdate::send(uuid, Metric::IncrementCounter(Counter::FragmentsCreated, self.frags_created), producer);
        MetricsUpdate::send(uuid, Metric::IncrementCounter(Counter::FragmentsSealed, self.frags_sealed), producer);
        MetricsUpdate::send(uuid, Metric::IncrementCounter(Counter::BlocksBuilt, self.blocks_built), producer);

        // Send current gauge values
        MetricsUpdate::send(
            uuid,
            Metric::SetGauge(Gauge::GatewayBlockHeight, self.current_block_height as f64),
            producer,
        );
        MetricsUpdate::send(
            uuid,
            Metric::SetGauge(Gauge::SimulationInFlightCount, self.simulation_in_flight_count as f64),
            producer,
        );
        MetricsUpdate::send(
            uuid,
            Metric::SetGauge(Gauge::SimulationQueueDepth, self.simulation_queue_depth as f64),
            producer,
        );
        MetricsUpdate::send(
            uuid,
            Metric::SetGauge(Gauge::ActiveTransactionsCount, self.active_tx_count as f64),
            producer,
        );
        MetricsUpdate::send(
            uuid,
            Metric::SetGauge(Gauge::TransactionPoolMemoryBytes, self.pool_memory_bytes as f64),
            producer,
        );

        // Send histogram summaries. each data point is already an average of 100 samples.
        if !self.sim_latencies.is_empty() {
            let avg_latency = self.sim_latencies.iter().sum::<f64>() / self.sim_latencies.len() as f64;
            MetricsUpdate::send(uuid, Metric::RecordHistogram(Histogram::SimulationLatencyMs, avg_latency), producer);
        }

        if !self.frag_tx_counts.is_empty() {
            let avg_tx_count = self.frag_tx_counts.iter().sum::<u64>() / self.frag_tx_counts.len() as u64;
            MetricsUpdate::send(
                uuid,
                Metric::RecordHistogram(Histogram::GatewayFragTxCount, avg_tx_count as f64),
                producer,
            );
        }

        if !self.frag_durations.is_empty() {
            let avg_duration = self.frag_durations.iter().sum::<f64>() / self.frag_durations.len() as f64;
            MetricsUpdate::send(uuid, Metric::RecordHistogram(Histogram::FragSealEndToEndMs, avg_duration), producer);
        }

        if !self.block_durations.is_empty() {
            let avg_duration = self.block_durations.iter().sum::<f64>() / self.block_durations.len() as f64;
            MetricsUpdate::send(
                uuid,
                Metric::RecordHistogram(Histogram::GatewayBlockBuildDurationMs, avg_duration),
                producer,
            );
        }

        // Reset counters (keep histograms for rolling window)
        self.sim_requests = 0;
        self.sim_results = 0;
        self.sim_errors = 0;
        self.frags_created = 0;
        self.frags_sealed = 0;
        self.blocks_built = 0;
    }
}

/// A metrics update is a message sent to the metrics consumer.
#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
pub struct MetricsUpdate {
    pub identifier: Uuid,
    pub t: Nanos,
    pub metric: Metric,
}

impl MetricsUpdate {
    /// Sends a metrics update to the producer.
    pub fn send(identifier: Uuid, metric: Metric, producer: &mut Producer<Self>) {
        let msg = Self { identifier, t: Nanos::now(), metric };
        producer.produce(&msg);
    }

    /// Sends a metrics update to the producer.
    pub fn send_ref(identifier: Uuid, metric: Metric, producer: &Producer<Self>) {
        let msg = Self { identifier, t: Nanos::now(), metric };
        producer.produce_without_first(&msg);
    }

    /// Sends a metrics update to the producer with a specific time.
    pub fn send_with_time(identifier: Uuid, t: Nanos, metric: Metric, producer: &mut Producer<Self>) {
        let msg = Self { identifier, t, metric };
        producer.produce(&msg);
    }
}

/// A metric is any value that can be tracked and reported.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[repr(u8)]
pub enum Metric {
    IncrementCounter(Counter, u64),
    SetGauge(Gauge, f64),
    RecordHistogram(Histogram, f64),
}

/// A counter is a metric that can be incremented and decremented.
/// It is used to count the number of occurrences of an event.
///
/// NOTE: we use `AsRefStr` to produce the metric name.
/// Example: `Counter::GatewayRpcIngressTxsTotal.as_ref()` => "gateway_rpc_ingress_txs_total".
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd, Ord, Eq, Serialize, Deserialize, AsRefStr)]
#[strum(serialize_all = "snake_case")]
pub enum Counter {
    /// Total amount of txs received via RPC
    GatewayRpcIngressTxsTotal,
    /// Total number of simulations sent to simulator
    SimulationRequestsSent,
    /// Total number of simulation results received
    SimulationResultsReceived,
    /// Total number of simulation errors
    SimulationErrors,
    /// Total number of fragments created
    FragmentsCreated,
    /// Total number of fragments sealed
    FragmentsSealed,
    /// Total number of blocks built
    BlocksBuilt,
}

/// A gauge is a metric that can be set to a specific value.
/// It is used to track the current value of a metric.
///
/// NOTE: we use `AsRefStr` to produce the metric name.
/// Example: `Gauge::GatewayBlockHeight(100).as_ref()` => "gateway_block_height".
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd, Ord, Eq, Serialize, Deserialize, AsRefStr)]
#[strum(serialize_all = "snake_case")]
pub enum Gauge {
    /// Current block height
    GatewayBlockHeight,
    /// Current number of simulations in flight
    SimulationInFlightCount,
    /// Current simulation queue depth
    SimulationQueueDepth,
    /// Current number of active transactions
    ActiveTransactionsCount,
    /// Current transaction pool memory usage in bytes
    TransactionPoolMemoryBytes,
}

/// A histogram is a metric that can be used to track the distribution of a value.
///
/// NOTE: we use `AsRefStr` to produce the metric name.
/// Example: `Histogram::GatewayFragSizeBytes(100).as_ref()` => "gateway_frag_size_bytes".
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd, Serialize, Deserialize, AsRefStr)]
#[strum(serialize_all = "snake_case")]
pub enum Histogram {
    /// Gateway frag tx count
    GatewayFragTxCount,
    /// Gateway block build duration in milliseconds
    GatewayBlockBuildDurationMs,
    /// Gateway simulation latency in milliseconds
    SimulationLatencyMs,
    /// Gateway frag sealing end-to-end time in milliseconds
    FragSealEndToEndMs,
    /// Gateway transaction processing end-to-end time in milliseconds
    TransactionProcessingEndToEndMs,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metric_serialization() {
        let metric = Counter::GatewayRpcIngressTxsTotal;
        let serialized = metric.as_ref();
        assert_eq!(serialized, r#"gateway_rpc_ingress_txs_total"#);

        let simulation_metric = Counter::SimulationRequestsSent;
        let simulation_serialized = simulation_metric.as_ref();
        assert_eq!(simulation_serialized, r#"simulation_requests_sent"#);
    }
}
