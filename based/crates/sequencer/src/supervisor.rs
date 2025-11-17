use std::sync::Arc;

use alloy_consensus::Transaction as _;
use bop_common::transaction::Transaction;
use reth_optimism_txpool::supervisor::{
    ExecutingDescriptor, InteropTxValidatorError, SupervisorClient, parse_access_list_items_to_inbox_entries,
};
use revm_primitives::B256;
use tracing::warn;

use crate::config::SuperVisorConfig;

#[derive(Debug, Clone)]
pub struct SupervisorValidator {
    client: SupervisorClient,
}

impl SupervisorValidator {
    pub async fn new(config: &SuperVisorConfig) -> Self {
        let client = SupervisorClient::builder(config.url.clone()).minimum_safety(config.safety_level).build().await;
        Self { client }
    }

    pub fn is_valid(&self, tx: &Arc<Transaction>, timestamp: u64) -> bool {
        let Some(access_list) = tx.access_list() else {
            return true;
        };

        let inbox_entries =
            parse_access_list_items_to_inbox_entries(access_list.iter()).copied().collect::<Vec<B256>>();

        let descriptor = ExecutingDescriptor::new(timestamp, None);
        let res = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(self.validate_messages(inbox_entries.as_slice(), descriptor));
        #[allow(clippy::match_single_binding)]
        match res {
            Ok(()) => true,
            Err(err) => {
                match err {
                    // // TODO: we should add reconnecting to supervisor in case of disconnect
                    // InteropTxValidatorError::SupervisorServerError(err) => {
                    //     warn!(%err, ?tx, "Supervisor error, skipping.");
                    //     false
                    // }
                    // InteropTxValidatorError::ValidationTimeout(_) => {
                    //     warn!(%err, ?tx, "Cross tx validation timed out, skipping.");
                    //     false
                    // }
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

impl SupervisorValidator {
    pub async fn validate_messages(
        &self,
        inbox_entries: &[B256],
        executing_descriptor: ExecutingDescriptor,
    ) -> Result<(), InteropTxValidatorError> {
        self.client.check_access_list(inbox_entries, executing_descriptor).await
    }
}

impl From<&SuperVisorConfig> for SupervisorValidator {
    fn from(value: &SuperVisorConfig) -> Self {
        tokio::runtime::Handle::current().block_on(Self::new(value))
    }
}
