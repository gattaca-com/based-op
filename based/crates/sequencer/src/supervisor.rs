use std::{sync::Arc};

use alloy_consensus::Transaction as _;
use bop_common::transaction::Transaction;
use kona_interop::SafetyLevel;
use reth_optimism_txpool::supervisor::SupervisorClient;

use crate::config::SuperVisorConfig;

#[derive(Debug, Clone)]
pub struct SupervisorValidator {
    client: SupervisorClient,
    safety: SafetyLevel,
}

impl SupervisorValidator {
    pub async fn new(config: &SuperVisorConfig) -> Self {
        let client = SupervisorClient::builder(config.url.clone())
            .minimum_safety(config.safety_level)
            .build()
            .await;
        Self { client, safety: config.safety_level }
    }

    pub fn is_valid(&self, tx: &Arc<Transaction>, timestamp: u64) -> bool {
        let Some(access_list) = tx.access_list() else {
            return true;
        };

        // SU TODO: Handle this

        true
    }
}

impl From<&SuperVisorConfig> for SupervisorValidator {
    fn from(value: &SuperVisorConfig) -> Self {
        tokio::runtime::Handle::current().block_on(Self::new(value))
    }
}
