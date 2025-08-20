use std::time::Duration;

use bop_common::{
    communication::Consumer,
    telemetry::{Telemetry, TelemetryUpdate, Tx, telemetry_queue},
};

use crate::metrics::Metrics;

/// Consumes telemetry updates from shared memory queues, and converts them into metrics.
pub struct MetricsConsumer {
    telemetry: Consumer<TelemetryUpdate>,
}

impl MetricsConsumer {
    /// Runs the metrics consumer, consuming telemetry updates from shared queues,
    /// and converting them into metrics.
    pub async fn run(&mut self) {
        loop {
            while let Some(update) = self.telemetry.try_consume() {
                self.process_update(update);
            }

            tokio::time::sleep(Duration::from_millis(50)).await
        }
    }

    /// Processes a telemetry update, converting it into metrics.
    fn process_update(&mut self, update: TelemetryUpdate) {
        match update.update {
            Telemetry::Tx(tx) => match tx {
                Tx::Included(_) => Metrics::increase_gateway_tx_included_total(),
                _ => todo!(),
            },
            _ => todo!(),
        }
    }
}

impl Default for MetricsConsumer {
    fn default() -> Self {
        Self { telemetry: telemetry_queue().into() }
    }
}
