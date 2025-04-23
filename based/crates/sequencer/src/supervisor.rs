use alloy_consensus::Transaction as _;
use bop_common::transaction::Transaction;
use jsonrpsee::http_client::{HttpClient, HttpClientBuilder};
use kona_interop::{ExecutingDescriptor, SafetyLevel};
use kona_rpc::{InteropTxValidator, InteropTxValidatorError};
use std::{sync::Arc, time::Duration};
use tracing::warn;

use crate::config::SuperVisorConfig;

#[derive(Debug, Clone)]
pub struct SupervisorValidator {
    client: HttpClient,
    safety: SafetyLevel,
}

impl SupervisorValidator {
    pub fn is_valid(&self, tx: &Arc<Transaction>, timestamp: u64) -> bool {
        let Some(access_list) = tx.access_list() else {
            return true;
        };

        let inbox_entries = SupervisorValidator::parse_access_list(access_list).cloned().collect::<Vec<_>>();

        if inbox_entries.is_empty() {
            return true;
        }

        let descriptor = ExecutingDescriptor::new(timestamp, None);
        let res = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(
            self.validate_messages(
                inbox_entries.as_slice(),
                self.safety,
                descriptor,
                Some(core::time::Duration::from_millis(100)),
            ),
        );
        match res {
            Ok(()) => true,
            Err(err) => {
                match err {
                    // TODO: we should add reconnecting to supervisor in case of disconnect
                    InteropTxValidatorError::SupervisorServerError(err) => {
                        warn!(%err, ?tx, "Supervisor error, skipping.");
                        false
                    }
                    InteropTxValidatorError::ValidationTimeout(_) => {
                        warn!(%err, ?tx, "Cross tx validation timed out, skipping.");
                        false
                    }
                    err => {
                        warn!(%err, ?tx, "Cross tx rejected.");
                        // It's possible that transaction invalid now, but would be valid later.
                        // We should keep limited queue for transactions that could become valid.
                        // We should have the limit to ensure that builder won't get overwhelmed.
                        false
                    }
                }
            }
        }
    }
}

impl InteropTxValidator for SupervisorValidator {
    type SupervisorClient = HttpClient;
    const DEFAULT_TIMEOUT: Duration = Duration::from_millis(100);

    fn supervisor_client(&self) -> &Self::SupervisorClient {
        &self.client
    }
}

impl From<&SuperVisorConfig> for SupervisorValidator {
    fn from(value: &SuperVisorConfig) -> Self {
        let client = HttpClientBuilder::default().build(value.url.clone()).expect("building supervisor http client");
        Self { client, safety: value.safety_level }
    }
}
