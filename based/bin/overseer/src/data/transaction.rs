use bop_common::{
    telemetry::order::{IncludedInFrag, Ingested, Tx},
    time::Nanos,
};
use ratatui::text::Text;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::collections::HasKey;
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct TransactionData {
    uuid: Uuid,
    pub updates: Vec<(Nanos, Tx)>,
}

impl TransactionData {
    pub fn new(uuid: Uuid, t: Nanos, update: Tx) -> Self {
        Self { uuid, updates: vec![(t, update)], ..Default::default() }
    }

    pub fn push(&mut self, t: Nanos, update: Tx) {
        self.updates.push((t, update))
    }

    pub fn in_block(&self, uuid: Uuid) -> Option<(usize, Uuid)> {
        self.updates.iter().find_map(|(_, u)| match u {
            Tx::Included(IncludedInFrag { frag, id_in_frag, .. }) if *frag == uuid => {
                Some((*id_in_frag as usize, self.uuid))
            }
            _ => None,
        })
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
                Tx::AddedToPool { .. } => active = true,
                Tx::RemovedFromPool { .. } => active = false,
                _ => {}
            }
        }

        active
    }

    pub fn included(&self) -> impl Iterator<Item = (&Nanos, &IncludedInFrag)> {
        self.updates.iter().filter_map(|(n, u)| if let Tx::Included(included) = u { Some((n, included)) } else { None })
    }

    pub fn pool_header() -> impl ExactSizeIterator<Item = Text<'static>> {
        ["Hash", "Sender", "Nonce", "Timestamp"].into_iter().map(|t| t.into())
    }

    pub fn to_pool_row(&self) -> Vec<Text<'_>> {
        let Some(ingested) = self.ingested() else {
            return vec![];
        };
        vec![
            ingested.hash.to_string()[0..6].to_string().into(),
            ingested.sender.to_string()[0..6].to_string().into(),
            ingested.nonce.to_string().into(),
            self.updates[0].0.with_fmt("%d %H:%M:%S%.3f").into(),
        ]
    }

    pub fn frag_table_header() -> impl ExactSizeIterator<Item = Text<'static>> {
        ["Hash", "Sender", "Nonce", "Payment", "Gas Used", "Simtime", "Timestamp"].into_iter().map(|t| t.into())
    }

    pub fn to_frag_row(&self) -> Vec<Text<'_>> {
        let Some(ingested) = self.ingested() else {
            return vec![];
        };

        let Some((_, included)) = self.included_in_frags().next() else {
            return vec![];
        };
        vec![
            ingested.hash.to_string()[0..6].to_string().into(),
            ingested.sender.to_string()[0..6].to_string().into(),
            ingested.nonce.to_string().into(),
            included.payment.to_string().into(),
            included.gas_used.to_string().into(),
            included.sim_time.to_string().into(),
            self.updates[0].0.with_fmt("%d %H:%M:%S%.3f").into(),
        ]
    }
}

impl HasKey for TransactionData {
    type Key = Uuid;

    fn key(&self) -> &Self::Key {
        &self.uuid
    }
}
