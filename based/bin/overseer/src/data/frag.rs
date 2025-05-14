use bop_common::{
    eth::MicroEth,
    telemetry::{Frag, order::IncludedInFrag},
    time::Nanos,
};
use ratatui::text::Text;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::collections::HasKey;
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct FragData {
    uuid: Uuid,
    pub updates: Vec<(Nanos, Frag)>,
    pub txs: Vec<Uuid>,
    pub payment: MicroEth,
    pub gas_used: u64,
    pub sim_time: Nanos,
}

impl FragData {
    pub fn new(uuid: Uuid, t: Nanos, update: Frag) -> Self {
        Self { uuid, updates: vec![(t, update)], ..Default::default() }
    }

    pub fn push(&mut self, t: Nanos, update: Frag) {
        self.updates.push((t, update))
    }
    pub fn add_tx(&mut self, uuid: Uuid, included: IncludedInFrag) {
        self.txs.push(uuid);
        self.payment += included.payment;
        self.gas_used += included.gas_used;
        self.sim_time += included.sim_time;
    }
    pub fn block_table_header() -> impl ExactSizeIterator<Item = Text<'static>> {
        ["Seq", "# Txs", "Payment", "Gas Used", "Simtime", "Timestamp"].into_iter().map(|t| t.into())
    }
}

impl HasKey for FragData {
    type Key = Uuid;

    fn key(&self) -> &Self::Key {
        &self.uuid
    }
}
