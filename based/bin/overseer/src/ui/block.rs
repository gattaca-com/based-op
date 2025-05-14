use crate::{
    collections::KeyedCircularBuffer,
    data::{block::BlockData, frag::FragData},
    prelude::*,
};

pub struct Block<'a> {
    block: &'a BlockData,
    frags: &'a KeyedCircularBuffer<FragData>,
    state: &'a mut TableState,
}
impl<'a> Block<'a> {
    pub fn new(block: &'a BlockData, frags: &'a KeyedCircularBuffer<FragData>, state: &'a mut TableState) -> Self {
        Self { block, frags, state }
    }

    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        let [top, bot] = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(12), Constraint::Fill(1)])
            .areas(area);

        let summary_info: Vec<Text> = self.block.to_row();
        let block_number = self.block.number;

        let mut block_summary_table = TableState::default();
        let header = BlockData::header();
        let n = header.len() - 1;

        block_summary_table.render(
            Some(format!("Block {block_number}")),
            header.take(n),
            std::iter::once(summary_info),
            frame,
            top,
        );

        let header = FragData::block_table_header();

        let rows =
            self.block.frags.iter().map(|h| self.frags.get(h).map(|f| f.to_block_table_row()).unwrap_or_default());
        self.state.render(None, header, rows, frame, bot);
    }
}
