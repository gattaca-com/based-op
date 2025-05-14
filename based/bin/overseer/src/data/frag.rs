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
    pub uuid: Uuid,
    pub updates: Vec<(Nanos, Frag)>,
    pub txs: Vec<Uuid>,
    pub payment: MicroEth,
    pub gas_used: u64,
    pub sim_time: Nanos,
}

impl FragData {
    pub fn new(t: Nanos, uuid: Uuid, update: Frag) -> Self {
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

    pub fn to_block_table_row(&self) -> Vec<Text<'_>> {
        let Frag::SorterStart { seq, .. } = &self.updates[0].1 else {
            return vec![];
        };
        let (payment, gas_used, n_txs) = self.frag_stats().unwrap_or_default();

        vec![
            seq.to_string().into(),
            n_txs.to_string().into(),
            payment.to_string().into(),
            gas_used.to_string().into(),
            self.sim_time.to_string().into(),
            self.updates[0].0.with_fmt("%d %H:%M:%S%.3f").into(),
        ]
    }

    pub fn block_number(&self) -> Option<u64> {
        self.updates.iter().find_map(|(_, u)| if let Frag::SorterStart { block, .. } = u { Some(*block) } else { None })
    }

    pub(crate) fn frag_stats(&self) -> Option<(MicroEth, u64, usize)> {
        self.updates.iter().find_map(|(_, u)| {
            if let Frag::SorterFinish { payment, gas_used, n_txs, .. } = u {
                Some((*payment, *gas_used, *n_txs))
            } else {
                None
            }
        })
    }
}

impl HasKey for FragData {
    type Key = Uuid;

    fn key(&self) -> &Self::Key {
        &self.uuid
    }
}
