use common::telemetry::{self, BundleFlags, TxFlags};
use ratatui::style::Modifier;

use super::CyclingTableState;
use crate::{
    data::{UIData, block::BlockData, bundle::BundleData, transaction::TransactionData},
    prelude::*,
};

pub struct Bundle<'a> {
    bundle: &'a BundleData,
    blocks: &'a KeyedVec<BlockData>,
    ui_data: &'a UIData,
    state: &'a mut CyclingTableState<B256>,
}

impl<'a> Bundle<'a> {
    pub fn new(bundle: &'a BundleData, state: &'a mut CyclingTableState<B256>, blocks: &'a KeyedVec<BlockData>, ui_data: &'a UIData) -> Self {
        Self { bundle, blocks, ui_data, state }
    }
    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        let flags = self.bundle.flags();
        let category_style = if flags.contains(BundleFlags::CexDex) {
            Style::default().red()
        } else if flags.contains(BundleFlags::EOB) {
            Style::default().yellow()
        } else if flags.contains(BundleFlags::TxHub) {
            Style::default().cyan()
        } else {
            Style::default()
        };
        let header = Text::from(self.bundle);
        let block_table_header = BlockData::header();
        let n_cols = block_table_header.len() + 3;
        let table_header = ["Id in Block", "Simtime", "Order Value"].into_iter().map(|t| t.into()).chain(block_table_header);

        let rows = self.bundle.updates.iter().filter_map(|(t, u)| match u {
            telemetry::order::Bundle::Included(included) => {
                let block = self.blocks.get(&included.block)?;
                if !self.ui_data.is_strat_visible(block.strategy_id()) {
                    return None;
                }

                let mut t = vec![included.id_in_block.to_string().into(), included.sim_time.to_string().into(), included.value.to_string().into()];
                t.extend(block.to_row());
                Some((block.uuid, t))
            }
            telemetry::order::Bundle::Replaced { .. } => {
                let mut t = vec![Text::from("REPLACED").add_modifier(Modifier::REVERSED).fg(Color::Red), Text::from(t.with_fmt("%d %H:%M:%S%.3f"))];
                for _ in 0..n_cols - 2 {
                    t.push("".into())
                }
                Some((B256::ZERO, t))
            }
            _ => None,
        });
        let header_len = header.lines.len();
        let [header_area, body] = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(header_len as u16 + 2), Constraint::Fill(1)])
            .areas(area);

        frame.render_widget(
            Paragraph::new(header)
                .block(Block::new().title(format!("Bundle {}", self.bundle.hash())).borders(Borders::ALL).border_style(category_style)),
            header_area,
        );
        self.state.render(None, table_header, rows, frame, body);
    }
}

pub struct Transaction<'a> {
    tx: &'a TransactionData,
    blocks: &'a KeyedVec<BlockData>,
    ui_data: &'a UIData,
    state: &'a mut CyclingTableState<B256>,
}

impl<'a> Transaction<'a> {
    pub fn new(tx: &'a TransactionData, state: &'a mut CyclingTableState<B256>, blocks: &'a KeyedVec<BlockData>, ui_data: &'a UIData) -> Self {
        Self { tx, blocks, ui_data, state }
    }
    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        let flags = self.tx.flags();
        let category_style = if flags.contains(TxFlags::CexDex) {
            Style::default().red()
        } else if flags.contains(TxFlags::TxHub) {
            Style::default().cyan()
        } else {
            Style::default()
        };
        let header = Text::from(self.tx);
        let block_table_header = BlockData::header();
        let n_cols = block_table_header.len() + 3;
        let table_header = ["Id in Block", "Simtime", "Order Value"].into_iter().map(|t| t.into()).chain(block_table_header);
        let rows = self.tx.updates.iter().filter_map(|(t, u)| match u {
            telemetry::order::Tx::Included(included) => {
                let block = self.blocks.get(&included.block)?;
                if !self.ui_data.is_strat_visible(block.strategy_id()) {
                    return None;
                }

                let mut t = vec![included.id_in_block.to_string().into(), included.sim_time.to_string().into(), included.value.to_string().into()];
                t.extend(block.to_row());
                Some((block.uuid, t))
            }
            telemetry::order::Tx::Removed { .. } => {
                let mut t = vec![Text::from("REMOVED").add_modifier(Modifier::REVERSED).fg(Color::Red), Text::from(t.with_fmt("%d %H:%M:%S%.3f"))];
                for _ in 0..n_cols - 2 {
                    t.push("".into())
                }
                Some((B256::ZERO, t))
            }
            _ => None,
        });

        let header_len = header.lines.len();
        let [header_area, body] = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(header_len as u16 + 2), Constraint::Fill(1)])
            .areas(area);

        frame.render_widget(
            Paragraph::new(header)
                .block(Block::new().title(format!("Transaction {}", self.tx.hash())).borders(Borders::ALL).border_style(category_style)),
            header_area,
        );
        self.state.render(None, table_header, rows, frame, body);
    }
}
