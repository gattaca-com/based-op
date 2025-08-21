use std::time::Duration;

use bop_common::{
    communication::Consumer,
    telemetry::{Telemetry, TelemetryUpdate, Tx, system::SystemNotification, telemetry_queue},
};
use tracing::trace;

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
                trace!(?update, "Received telemetry update");
                self.process_update(update);
            }

            tokio::time::sleep(Duration::from_millis(50)).await
        }
    }

    /// Processes a telemetry update, converting it into metrics.
    fn process_update(&mut self, update: TelemetryUpdate) {
        match update.update {
            Telemetry::Tx(tx) => match tx {
                Tx::AddedToPool => Metrics::increase_gateway_tx_added_to_pool_total(),
                Tx::Included(_) => Metrics::increase_gateway_tx_included_total(),
                _ => {
                    // TODO
                }
            },
            Telemetry::System(system) => match system {
                SystemNotification::StateChanged(state) => Metrics::set_sequencer_state(state),
                SystemNotification::BlockSync(block_number, _gas_used) => {
                    Metrics::set_block_sync_block_number(block_number)
                }
                SystemNotification::Sorting(block_number) => Metrics::set_sorting_block_number(block_number),
                _ => {
                    // TODO
                }
            },
            _ => {
                // TODO
            }
        }
    }
}

impl Default for MetricsConsumer {
    fn default() -> Self {
        Self { telemetry: telemetry_queue().into() }
    }
}
