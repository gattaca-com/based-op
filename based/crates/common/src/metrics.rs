use revm_primitives::Address;
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
    /// Total number of simulations sent to simulator
    SimulationRequestsSent,
    /// Total number of simulation results received
    SimulationResultsReceived,
    /// Total number of simulation errors
    SimulationErrors,

    /// Total number of requests received by the portal
    PortalTotalRequests,
    /// Total number of requests received by the portal
    PortalApiRequests,
    /// Total number of requests received by the portal
    EngineApiRequests,
    /// Total number of requests received by the portal
    RegistryApiRequests,
    /// Total number of requests received by the portal
    OpNodeApiRequests,
    /// Total number of requests received by the portal
    FallbackApiRequests,
    /// Total number of payloads served from gateway
    PayloadsServedFromGateway,
    /// Total number of payloads served from fallback
    PayloadsServedFromFallback,
    /// Total number of payloads that failed to be served
    PayloadServeFailed,
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
    /// Current transaction pool memory usage in bytes
    TransactionPoolMemoryBytes,

    /// Current portal->gateway ping latency in milliseconds
    PortalGatewayPingLatencyMs(Address),
    /// Current portal->gateway registry address
    PortalCurrentGatewayRegistryAddress(Address),
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
        let metric = Counter::SimulationRequestsSent;
        let serialized = metric.as_ref();
        assert_eq!(serialized, r#"simulation_requests_sent"#);

        let simulation_metric = Counter::SimulationRequestsSent;
        let simulation_serialized = simulation_metric.as_ref();
        assert_eq!(simulation_serialized, r#"simulation_requests_sent"#);
    }
}
