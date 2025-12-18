use alloy_primitives::{Address, B256};
use bop_common::{
    telemetry::order::{Ingested, TransactionInclusion, Tx},
    time::{Duration, Nanos, Repeater},
};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    text::Text,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::Data;
use crate::{collections::HasKey, statistics::Statistics, ui::ToRow};
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

    pub fn included_in_frags(&self) -> impl Iterator<Item = (&Nanos, &TransactionInclusion)> {
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
    pub sent_timestamp: Nanos,
    pub wallet: Address,
    pub nonce: u64,
    pub hash: Option<B256>,
    pub block: Option<u64>,
    pub receipt_timestamp: Option<Nanos>,
}
impl SpammedTx {
    pub fn new(
        sent_timestamp: Nanos,
        wallet: Address,
        nonce: u64,
        hash: Option<B256>,
        block: Option<u64>,
        receipt_timestamp: Option<Nanos>,
    ) -> Self {
        Self { wallet, hash, nonce, block, sent_timestamp, receipt_timestamp }
    }

    pub fn header() -> impl ExactSizeIterator<Item = Text<'static>> {
        ["Timestamp", "Nonce", "Hash", "Block", "Latency"].into_iter().map(|t| t.into())
    }

    pub fn latency(&self) -> Duration {
        Duration::from(self.receipt_timestamp.unwrap() - self.sent_timestamp)
    }
}

impl HasKey for SpammedTx {
    type Key = Nanos;

    fn key(&self) -> &Self::Key {
        &self.sent_timestamp
    }
}

impl ToRow for SpammedTx {
    fn to_row(&self, _data: &Data) -> Vec<Text<'_>> {
        vec![
            self.sent_timestamp.with_fmt("%d %H:%M:%S%.3f").into(),
            self.nonce.to_string().into(),
            self.hash.map(|t| t.to_string()).unwrap_or_default().into(),
            self.block.map(|t| t.to_string()).unwrap_or_default().into(),
            self.receipt_timestamp.map(|r| (r - self.sent_timestamp).to_string()).unwrap_or_default().into(),
        ]
    }
}

#[derive(Clone, Debug)]
pub struct SpamData {
    latency: Statistics<Duration>,
    point_registrator: Repeater,
}

impl Default for SpamData {
    fn default() -> Self {
        Self {
            latency: Statistics::new("Latency".to_string(), 4096, 256, Duration::ZERO),
            point_registrator: Repeater::every(Duration::from_secs(1)),
        }
    }
}

impl SpamData {
    pub fn track(&mut self, latency: Duration) {
        self.latency.track(latency);
        if self.point_registrator.fired() {
            self.latency.register_datapoint(0, false);
        }
    }

    pub fn latencies(&self) -> (Duration, Duration, Duration, Duration) {
        (self.latency.min(), self.latency.avg(), self.latency.med(), self.latency.max())
    }

    pub fn report(&self, frame: &mut Frame, area: Rect) {
        let [top, bottom] = Layout::vertical([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(area);

        self.latency.report("", frame, top);
        self.latency.report_msg_per_sec(frame, bottom);
    }
}
