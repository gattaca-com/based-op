use alloy_consensus::Transaction as _;
use alloy_primitives::B256;
use bop_common::transaction::Transaction;
use op_alloy_consensus::interop::SafetyLevel;
use reqwest::Url;
use reth_optimism_txpool::supervisor::{
    ExecutingDescriptor, InteropTxValidatorError, SupervisorClient, parse_access_list_items_to_inbox_entries,
};
use tracing::warn;

#[derive(Clone, Debug)]
pub struct SuperVisorConfig {
    pub url: Url,
    pub safety_level: SafetyLevel,
}

#[derive(Debug, Clone)]
pub(crate) struct SupervisorValidator {
    client: SupervisorClient,
}

impl SupervisorValidator {
    pub(crate) async fn new(config: &SuperVisorConfig) -> Self {
        let client = SupervisorClient::builder(config.url.clone()).minimum_safety(config.safety_level).build().await;
        Self { client }
    }

    /// Validates a cross-chain transaction.
    pub(crate) async fn validate(&self, tx: &Transaction, timestamp: u64) -> Result<(), InteropTxValidatorError> {
        let Some(access_list) = tx.access_list() else {
            return Ok(());
        };

        let inbox_entries =
            parse_access_list_items_to_inbox_entries(access_list.iter()).copied().collect::<Vec<B256>>();

        let descriptor = ExecutingDescriptor::new(timestamp, None);

        if let Err(err) = self.validate_messages(inbox_entries.as_slice(), descriptor).await {
            // TODO: Deal with reconnects. This will require `&mut self` here so it's going to be difficult in the RPC
            // context. Maybe the validator should be a separate actor.
            warn!(?err, ?tx, "Cross-chain transaction rejected");
            // It's possible that transaction invalid now, but would be valid later.
            // We should keep limited queue for transactions that could become valid.
            // We should have the limit to ensure that builder won't get overwhelmed.
            return Err(err);
        }

        Ok(())
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
