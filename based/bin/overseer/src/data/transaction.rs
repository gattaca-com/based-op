use alloy_primitives::{Address, B256};
use bop_common::{
    telemetry::order::{IncludedInFrag, Ingested, Tx},
    time::Nanos,
};
use ratatui::text::Text;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::Data;
use crate::{collections::HasKey, ui::ToRow};
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct TransactionData {
    uuid: Uuid,
    pub updates: Vec<(Nanos, Tx)>,
}

impl TransactionData {
    pub fn new(uuid: Uuid, t: Nanos, update: Tx) -> Self {
        Self { uuid, updates: vec![(t, update)] }
    }

    pub fn push(&mut self, t: Nanos, update: Tx) {
        self.updates.push((t, update))
    }

    pub fn ingested(&self) -> Option<&Ingested> {
        self.updates.iter().find_map(|(_, u)| match u {
            Tx::Ingested(ingested) => Some(ingested),
            _ => None,
        })
    }

    pub fn included_in_frags(&self) -> impl Iterator<Item = (&Nanos, &IncludedInFrag)> {
        self.updates.iter().filter_map(|(t, u)| match u {
            Tx::Included(included) => Some((t, included)),
            _ => None,
        })
    }

    #[allow(dead_code)]
    pub(crate) fn active(&self) -> bool {
        let mut active = false;
        for (_, u) in &self.updates {
            match u {
                Tx::AddedToPool => active = true,
                Tx::RemovedFromPool => active = false,
                _ => {}
            }
        }

        active
    }

    pub fn pool_header() -> impl ExactSizeIterator<Item = Text<'static>> {
        ["Timestamp", "Hash", "Sender", "Nonce"].into_iter().map(|t| t.into())
    }

    #[allow(dead_code)]
    pub fn frag_table_header() -> impl ExactSizeIterator<Item = Text<'static>> {
        ["Timestamp", "Hash", "Sender", "Nonce", "Payment", "Gas", "Simtime"].into_iter().map(|t| t.into())
    }

    #[allow(dead_code)]
    pub fn to_frag_row(&self) -> Vec<Text<'_>> {
        let Some(ingested) = self.ingested() else {
            return vec![];
        };

        let Some((_, included)) = self.included_in_frags().next() else {
            return vec![];
        };
        vec![
            self.updates[0].0.with_fmt("%d %H:%M:%S%.3f").into(),
            ingested.hash.to_string()[0..6].to_string().into(),
            ingested.sender.to_string()[0..6].to_string().into(),
            ingested.nonce.to_string().into(),
            included.payment.to_string().into(),
            included.gas_used.to_string().into(),
            included.sim_time.to_string().into(),
        ]
    }
}

impl HasKey for TransactionData {
    type Key = Uuid;

    fn key(&self) -> &Self::Key {
        &self.uuid
    }
}

impl ToRow for TransactionData {
    fn to_row(&self, _data: &Data) -> Vec<Text<'_>> {
        if !self.active() {
            return vec![];
        }
        let Some(ingested) = self.ingested() else {
            return vec![];
        };

        vec![
            self.updates[0].0.with_fmt("%d %H:%M:%S%.3f").into(),
            ingested.hash.to_string()[0..6].to_string().into(),
            ingested.sender.to_string()[0..6].to_string().into(),
            ingested.nonce.to_string().into(),
        ]
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct SpammedTx {
    pub wallet: Address,
    pub hash: B256,
    pub nonce: u64,
    pub block: u64,
    pub sent_timestamp: Nanos,
    pub receipt_timestamp: Nanos,
}
impl SpammedTx {
    pub fn new(
        wallet: Address,
        hash: B256,
        nonce: u64,
        block: u64,
        sent_timestamp: Nanos,
        receipt_timestamp: Nanos,
    ) -> Self {
        Self { wallet, hash, nonce, block, sent_timestamp, receipt_timestamp }
    }

    pub fn header() -> impl ExactSizeIterator<Item = Text<'static>> {
        ["Timestamp", "Hash", "Nonce", "Block", "Latency"].into_iter().map(|t| t.into())
    }
}

impl ToRow for SpammedTx {
    fn to_row(&self, _data: &Data) -> Vec<Text<'_>> {
        vec![
            self.sent_timestamp.with_fmt("%d %H:%M:%S%.3f").into(),
            self.hash.to_string().into(),
            self.nonce.to_string().into(),
            self.block.to_string().into(),
            (self.receipt_timestamp - self.sent_timestamp).to_string().into(),
        ]
    }
}
