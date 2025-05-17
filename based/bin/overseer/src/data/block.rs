use bop_common::{eth::MicroEth, time::Nanos};
use ratatui::text::Text;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::collections::HasKey;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct BlockData {
    // usually uuid but for submission we generate another one so it doesn't
    // overwrite the strat block
    pub number: u64,
    pub timestamp: Nanos,
    pub frags: Vec<Uuid>,
    pub we_sequenced: bool,
    pub gas_used: u64,
    pub payment: MicroEth,
    pub n_txs: usize,
    pub sealed: bool
}

impl BlockData {
    pub fn new(number: u64, we_sequenced: bool, timestamp: Nanos) -> Self {
        Self { number, frags: vec![], we_sequenced, timestamp, ..Default::default() }
    }
    pub fn push(&mut self, frag: Uuid, payment: MicroEth, gas_used: u64, n_txs: usize) {
        if self.sealed {
            return
        }
        self.frags.push(frag);
        self.payment += payment;
        self.gas_used += gas_used;
        self.n_txs += n_txs;
    }

    pub fn header() -> impl ExactSizeIterator<Item = Text<'static>> {
        ["Number", "# Frags", "# Txs", "Gas Used", "Payment", "Timestamp"].into_iter().map(|t| t.into())
    }

    pub fn to_row(&self) -> Vec<Text<'_>> {
        let t_start = self.timestamp;
        vec![
            self.number.to_string().into(),
            self.frags.len().to_string().into(),
            self.n_txs.to_string().into(),
            self.gas_used.to_string().into(),
            self.payment.to_string().into(),
            t_start.with_fmt("%d %H:%M:%S%.3f").into(),
        ]
    }
}

impl HasKey for BlockData {
    type Key = u64;

    fn key(&self) -> &Self::Key {
        &self.number
    }
}
