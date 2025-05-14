use crate::{
    data::{Data, block::BlockData},
    prelude::*,
    ui,
};

pub struct BuiltBlocks {}

impl BuiltBlocks {
    pub fn render(&mut self, data: &mut Data, frame: &mut Frame, area: Rect) {
        use crate::data::BuiltBlocksMode::*;
        let mut mode = std::mem::take(&mut data.built_blocks_tab.mode);
        match &mut mode {
            BlocksTable(state) => {
                let blocks = data
                    .blocks_visible()
                    .map(|block| {
                        let row: Vec<Text> = block.to_row();
                        (block.uuid, row)
                    })
                    .collect::<Vec<_>>();

                state.render(None, BlockData::header(), blocks.into_iter(), frame, area);
            }

            DetailsTable(block, state, _) => {
                if data.displayed().blocks.get(block).is_none() {
                    data.display_prev_slot();
                }
                let block = data.displayed().blocks.get(block).expect("selected a block that doesn't exist");
                ui::block::Block::new(block, &data.displayed().bundles, &data.displayed().transactions, state, &data.ui_data).render(frame, area);
            }
            Order(hash, state, _) => {
                if let Some(bundle) = data.displayed().bundles.get(hash) {
                    ui::order::Bundle::new(bundle, state, &data.displayed().blocks, &data.ui_data).render(frame, area)
                } else if let Some(tx) = data.displayed().transactions.get(hash) {
                    ui::order::Transaction::new(tx, state, &data.displayed().blocks, &data.ui_data).render(frame, area)
                } else {
                    data.display_prev_slot();
                }
            }
        }
        data.built_blocks_tab.mode = mode;
    }
}
