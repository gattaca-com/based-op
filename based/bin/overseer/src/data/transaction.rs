use alloy_primitives::Address;
use bop_common::{
    eth::MicroEth,
    telemetry::order::{IncludedInFrag, Ingested, Tx},
    time::Nanos,
};
use ratatui::text::{Span, Text};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{collections::HasKey, utils::fmt_with_pre_pad_till_9};
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
        self.updates.iter().find_map(|(t, u)| match u {
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
                Tx::Removed { .. } => active = false,
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
        let t_start = self.timestamp;
        let ingested = self.ingested();
        vec![
            ingested.hash.to_string()[0..6].to_string(),
            ingested.sender.to_string()[0..6].to_string(),
            ingested.nonce.to_string().into(),
            self.updates[0].0.with_fmt("%d %H:%M:%S%.3f").into(),
        ]
    }

    pub fn frag_table_header() -> impl ExactSizeIterator<Item = Text<'static>> {
        ["Hash", "Sender", "Nonce", "Payment", "Gas Used", "Simtime", "Timestamp"].into_iter().map(|t| t.into())
    }

    pub fn to_frag_row(&self) -> Vec<Text<'_>> {
        let t_start = self.timestamp;

        let ingested = self.ingested();

        let Some(included) = self.included_in_frags().next() else {
            return;
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
impl From<&TransactionData> for Text<'_> {
    fn from(value: &TransactionData) -> Self {
        let mut header = vec![];
        let first_seen = value.updates[0].0;

        let mut sender = Address::default();
        let mut tob_value = MicroEth::default();
        let mut n_ingested = 0;
        let mut first_ingested = Nanos::MAX;
        let mut last_ingested = Default::default();
        let mut n_added_to_pool = 0;
        let mut first_added_to_pool = Nanos::MAX;
        let mut last_added_to_pool = Default::default();
        let mut n_added_in_bundle = 0;
        let mut first_added_in_bundle = Nanos::MAX;
        let mut last_added_in_bundle = Default::default();
        let mut n_removed_with_bundle = 0;
        let mut first_removed_with_bundle = Nanos::MAX;
        let mut last_removed_with_bundle = Default::default();
        let mut n_included_in_block = 0;
        let mut first_included_in_block = Nanos::MAX;
        let mut last_included_in_block = Default::default();
        let mut max_included_value = MicroEth(0);
        let mut min_included_value = MicroEth(u32::MAX);
        let mut n_removed = 0;
        let mut first_removed = Nanos::MAX;
        let mut last_removed = Default::default();

        let n_submitted = value.submitted.len();
        let first_submitted = value.submitted.first().map(|(t, _)| *t).unwrap_or_default();
        let last_submitted = value.submitted.first().map(|(t, _)| *t).unwrap_or_default();
        for (el, update) in &value.updates {
            let el = *el;
            let (first, last) = match update {
                Tx::Ingested(Ingested { sender: s, .. }) => {
                    n_ingested += 1;
                    sender = *s;
                    (&mut first_ingested, &mut last_ingested)
                }
                Tx::AddedToPool { .. } => {
                    n_added_to_pool += 1;
                    (&mut first_added_to_pool, &mut last_added_to_pool)
                }
                Tx::Included(IncludedInFrag { payment: value_micro_eth, .. }) => {
                    n_included_in_block += 1;
                    max_included_value = max_included_value.max(*value_micro_eth);
                    min_included_value = min_included_value.min(*value_micro_eth);
                    (&mut first_included_in_block, &mut last_included_in_block)
                }
                Tx::Removed { .. } => {
                    n_removed += 1;
                    (&mut first_removed, &mut last_removed)
                }
            };
            *first = (*first).min(el);
            *last = el;
        }

        header.push(Span::raw(format!("TOB value: {tob_value}")).into());
        header.push(Span::raw(format!("Sender:    {sender}")).into());
        header.push(Span::raw(format!("First seen {} ", first_seen)).into());
        header.push(
            Span::raw(if n_ingested != 0 {
                format!(
                    "Ingested:          first {}, last {}, n: {}",
                    fmt_with_pre_pad_till_9(&(first_ingested - first_seen)),
                    fmt_with_pre_pad_till_9(&(last_ingested - first_seen)),
                    n_ingested
                )
            } else {
                "Ingested:         first 0, last 0, n: 0".to_string()
            })
            .into(),
        );
        header.push(
            Span::raw(if n_added_to_pool != 0 {
                format!(
                    "AddedToPool:       first {}, last {}, n: {}",
                    fmt_with_pre_pad_till_9(&(first_added_to_pool - first_seen)),
                    fmt_with_pre_pad_till_9(&(last_added_to_pool - first_seen)),
                    n_added_to_pool
                )
            } else {
                "AddedToPool:       first 0, last 0, n: 0".to_string()
            })
            .into(),
        );
        header.push(
            Span::raw(if n_added_in_bundle != 0 {
                format!(
                    "AddedInBundle:     first {}, last {}, n: {}",
                    fmt_with_pre_pad_till_9(&(first_added_in_bundle - first_seen)),
                    fmt_with_pre_pad_till_9(&(last_added_in_bundle - first_seen)),
                    n_added_in_bundle
                )
            } else {
                "AddedInBundle:     first 0, last 0, n: 0".to_string()
            })
            .into(),
        );
        header.push(
            Span::raw(if n_removed_with_bundle != 0 {
                format!(
                    "RemovedWithBundle: first {}, last {}, n: {}",
                    fmt_with_pre_pad_till_9(&(first_removed_with_bundle - first_seen)),
                    fmt_with_pre_pad_till_9(&(last_removed_with_bundle - first_seen)),
                    n_removed_with_bundle
                )
            } else {
                "RemovedWithBundle: first 0, last 0, n: 0".to_string()
            })
            .into(),
        );
        header.push(
            Span::raw(if n_removed != 0 {
                format!(
                    "Removed:           first {}, last {}, n: {}",
                    fmt_with_pre_pad_till_9(&(first_removed - first_seen)),
                    fmt_with_pre_pad_till_9(&(last_removed - first_seen)),
                    n_removed
                )
            } else {
                "Removed:           first 0, last 0, n: 0".to_string()
            })
            .into(),
        );
        header.push(
            Span::raw(if n_included_in_block != 0 {
                format!(
                    "IncludedInBlock:   first {}, last {}, n: {} -- value: max {}, min {}",
                    fmt_with_pre_pad_till_9(&(first_included_in_block - first_seen)),
                    fmt_with_pre_pad_till_9(&(last_included_in_block - first_seen)),
                    n_included_in_block,
                    max_included_value,
                    min_included_value
                )
            } else {
                "IncludedInBlock:   first 0, last 0, n: 0".to_string()
            })
            .into(),
        );

        header.push(
            Span::raw(if n_included_in_block != 0 {
                format!(
                    "Submitted:          first {}, last {}, n: {}",
                    fmt_with_pre_pad_till_9(&(first_submitted - first_seen)),
                    fmt_with_pre_pad_till_9(&(last_submitted - first_seen)),
                    n_submitted,
                )
            } else {
                "Submitted:          first 0, last 0, n: 0".to_string()
            })
            .into(),
        );
        header.into()
    }
}

impl HasKey for TransactionData {
    type Key = Uuid;

    fn key(&self) -> &Self::Key {
        &self.uuid
    }
}
