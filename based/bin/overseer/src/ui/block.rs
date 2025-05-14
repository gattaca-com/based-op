use ratatui::style::Modifier;

use crate::{
    data::{UIData, block::BlockData, transaction::TransactionData},
    prelude::*,
};

pub struct Block<'a> {
    block: &'a BlockData,
    transactions: &'a KeyedVec<TransactionData>,
    _ui_data: &'a UIData,
    state: &'a mut CyclingTableState<Uuid>,
}
impl<'a> Block<'a> {
    pub fn new(
        block: &'a BlockData,
        transactions: &'a KeyedVec<TransactionData>,
        state: &'a mut CyclingTableState<Uuid>,
        ui_data: &'a UIData,
    ) -> Self {
        Self { block, transactions, _ui_data: ui_data, state }
    }

    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        let [top, bot] = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(12), Constraint::Fill(1)])
            .areas(area);

        let summary_info: Vec<Text> = self.block.to_row();
        let block_uuid = self.block.uuid;

        let mut block_summary_table = CyclingTableState::default();
        let header = BlockData::header();
        let n = header.len() - 1;

        block_summary_table.render(
            Some(format!("Block {block_uuid}")),
            header.take(n),
            std::iter::once((block_uuid, summary_info)),
            frame,
            top,
        );

        let header = ["ID", "Tx", "Value", "Sim Time", "Timestamp", "Hash", "Replacement UUID/Sender"]
            .iter()
            .map(|t| Text::from(t.to_string()).bold());

        let rows = self.block.orders.iter().map(|h| {
            if let Some(tx) = self.transactions.get(h) {
                let (t, included_in_block) = tx.included_in_block(block_uuid).expect("tx in block was removed");
                (
                    *tx.hash(),
                    vec![
                        Text::from(included_in_block.id_in_block.to_string()),
                        Text::from(included_in_block.value.to_string()),
                        Text::from(included_in_block.sim_time.to_string()),
                        Text::from(t.with_fmt("%d %H:%M:%S%.3f")),
                        Text::from(h.to_string()[0..6].to_string()),
                        Text::from(if let Some(sender) = tx.sender() {
                            sender.to_string()[0..6].to_string()
                        } else {
                            "".to_string()
                        })
                        .style(style),
                    ],
                )
            }
        });
        self.state.render(None, header, rows, frame, bot);
    }
}
