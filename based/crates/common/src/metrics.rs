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
    IncrementCounter(Counter),
    SetGauge(Gauge),
    RecordHistogram(Histogram),
}

/// A counter is a metric that can be incremented and decremented.
/// It is used to count the number of occurrences of an event.
///
/// NOTE: we use `AsRefStr` to produce the metric name.
/// Example: `Counter::GatewayIngressTxsTotal.as_ref()` => "gateway_ingress_txs_total".
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, AsRefStr)]
#[strum(serialize_all = "snake_case")]
pub enum Counter {
    /// Total amount of txs received via RPC
    GatewayRpcIngressTxsTotal,
    GatewayFragTxCount,
}

/// A gauge is a metric that can be set to a specific value.
/// It is used to track the current value of a metric.
///
/// NOTE: we use `AsRefStr` to produce the metric name.
/// Example: `Gauge::GatewayBlockHeight(100).as_ref()` => "gateway_block_height".
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, AsRefStr)]
#[strum(serialize_all = "snake_case")]
pub enum Gauge {
    GatewayBlockHeight(u64),
}

impl Gauge {
    /// Returns the value of the gauge as a f64, useful for setting gauges.
    pub const fn value(&self) -> f64 {
        match self {
            Gauge::GatewayBlockHeight(block_num) => *block_num as f64,
        }
    }
}

/// A histogram is a metric that can be used to track the distribution of a value.
///
/// NOTE: we use `AsRefStr` to produce the metric name.
/// Example: `Histogram::GatewayFragSizeBytes(100).as_ref()` => "gateway_frag_size_bytes".
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, AsRefStr)]
#[strum(serialize_all = "snake_case")]
pub enum Histogram {
    GatewayFragSizeBytes(u64),
    GatewayFragTxCount(u64),
    GatewayBlockBuildDurationMs(f64),
}

impl Histogram {
    /// Returns the value of the histogram as a f64, useful for recording histograms.
    pub const fn value(&self) -> f64 {
        match self {
            Histogram::GatewayFragSizeBytes(bytes) => *bytes as f64,
            Histogram::GatewayFragTxCount(tx_count) => *tx_count as f64,
            Histogram::GatewayBlockBuildDurationMs(duration_ms) => *duration_ms,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metric_serialization() {
        let metric = Counter::GatewayRpcIngressTxsTotal;
        let serialized = metric.as_ref();
        assert_eq!(serialized, r#"gateway_rpc_ingress_txs_total"#);
    }
}
