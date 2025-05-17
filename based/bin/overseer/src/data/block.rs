use bop_common::{eth::MicroEth, time::Nanos};
use ratatui::{style::Style, text::Text};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{collections::HasKey, utils::empty_if_default};

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
    pub sealed: bool,
}

impl BlockData {
    pub fn new(number: u64, we_sequenced: bool, timestamp: Nanos) -> Self {
        Self { number, frags: vec![], we_sequenced, timestamp, ..Default::default() }
    }
    pub fn push(&mut self, frag: Uuid, payment: MicroEth, gas_used: u64, n_txs: usize) {
        if self.sealed {
            tracing::info!("already sealed {}", self.number);
            return;
        }
        self.frags.push(frag);
        self.payment += payment;
        self.gas_used += gas_used;
        self.n_txs += n_txs;
    }

    pub fn header() -> impl ExactSizeIterator<Item = Text<'static>> {
        ["Us", "Number", "# F", "# T", "Gas", "Payment", "Timestamp"].into_iter().map(|t| t.into())
    }

    pub fn to_row(&self) -> Vec<Text<'_>> {
        let t_start = self.timestamp;
        // we remove the first frag which is anyway the one with all the force inclusion
        let frags = self.frags.len().saturating_sub(1);
        let (style, us_str) = if self.we_sequenced {
            (Style::new().fg(ratatui::style::Color::Green), "X".to_string())
        } else {
            (Style::default(), "".to_string())
        };
        vec![
            Text::from(us_str).style(style),
            Text::from(self.number.to_string()).style(style),
            Text::from(empty_if_default(frags)).style(style),
            Text::from(empty_if_default(self.n_txs)).style(style),
            Text::from(empty_if_default(self.gas_used)).style(style),
            Text::from(empty_if_default(self.payment)).style(style),
            Text::from(t_start.with_fmt("%d %H:%M:%S%.3f")).style(style),
        ]
    }

    pub fn reset(&mut self) {
        self.frags.clear();
        self.payment = MicroEth(0);
        self.gas_used = 0;
        self.n_txs = 0;
    }
}

impl HasKey for BlockData {
    type Key = u64;

    fn key(&self) -> &Self::Key {
        &self.number
    }
}
