use std::time::Duration;

use bop_common::{
    communication::Consumer,
    telemetry::{Frag, Telemetry, TelemetryUpdate, Tx, system::SystemNotification, telemetry_queue},
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
    pub async fn run(mut self) {
        loop {
            while let Some(update) = self.telemetry.try_consume() {
                trace!(?update, "Received telemetry update");
                self.process_update(update);
            }

            tokio::time::sleep(Duration::from_millis(20)).await
        }
    }

    /// Processes a telemetry update, converting it into metrics.
    fn process_update(&mut self, update: TelemetryUpdate) {
        match update.update {
            Telemetry::Tx(tx) => match tx {
                Tx::AddedToPool => Metrics::increase_gateway_tx_added_to_pool_total(),
                Tx::RemovedFromPool => Metrics::increase_gateway_tx_removed_from_pool_total(),
                Tx::Included(_) => Metrics::increase_gateway_tx_included_total(),
                Tx::Ingested(_) => Metrics::increase_gateway_tx_ingested_total(),
            },
            Telemetry::System(system) => match system {
                SystemNotification::StateChanged(state) => Metrics::set_sequencer_state(state),
                SystemNotification::BlockSync(block_num, _) => Metrics::set_block_sync_block_number(block_num),
                SystemNotification::Sorting(block_num) => Metrics::set_sorting_block_number(block_num),
                SystemNotification::BuildStop(block_num) => Metrics::set_build_stop_block_number(block_num),
                SystemNotification::NewPayload(block_num) => Metrics::set_new_payload_block_number(block_num),
                SystemNotification::GetPayload(block_num) => Metrics::set_get_payload_block_number(block_num),
                SystemNotification::ForkChoiceUpdate(_block_hash) => { /* event skipped*/ }
            },
            Telemetry::Frag(frag) => match frag {
                Frag::SorterStart { block, available_value, .. } => {
                    Metrics::increase_frag_sorter_start_total();
                    Metrics::set_frag_current_block_number(block);
                    Metrics::record_frag_available_value(available_value.into());
                }
                Frag::SorterFinish { payment, best_order_value, n_txs, gas_used, .. } => {
                    Metrics::increase_frag_sorter_finish_total();
                    Metrics::record_frag_payment(payment.into());
                    Metrics::record_frag_gas_used(gas_used);
                    Metrics::record_frag_transaction_count(n_txs);
                    Metrics::record_frag_best_order_value(best_order_value.into());
                }
                Frag::Commit => Metrics::increase_sequencer_commit_frag_total(),
            },
        }
    }
}

impl Default for MetricsConsumer {
    fn default() -> Self {
        Self { telemetry: telemetry_queue().into() }
    }
}
