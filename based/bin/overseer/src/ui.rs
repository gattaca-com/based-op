use std::fmt::Write;

use auto_impl::auto_impl;
use ratatui::{
    style::{Modifier, Styled},
    text::Line,
    widgets::{Row, TableState as RTableState},
};

use crate::{data::Data, prelude::*};
pub mod plot;

#[auto_impl(&)]
pub trait ToRow {
    fn to_row<'a>(&'a self, data: &Data) -> Vec<Text<'a>>;
}

#[derive(Debug, Clone, Default)]
pub struct TableState {
    state: RTableState,
    colwidths: Vec<u16>,
}

impl TableState {
    pub fn render<'a, 'b, S: Into<Text<'b>>, R: ToRow>(
        &mut self,
        title: Option<String>,
        header: impl Iterator<Item = S>,
        rows: impl Iterator<Item = R>,
        data: &Data,
        frame: &mut Frame,
        area_: Rect,
    ) {
        let mut height_avail = area_.height;

        let mut b = Block::default().borders(Borders::ALL);
        if let Some(title) = title {
            b = b.title(title);
        }
        let area = b.inner(area_);
        frame.render_widget(b, area_);

        let mut drawable_header = vec![];
        let mut header_height = 0;
        for (ih, h) in header.enumerate() {
            let mut s = String::new();
            let h = h.into();
            let mut height = 0;
            for line in &h.lines {
                let _ = writeln!(&mut s, "▏ {} ", line);
                height += 1;
                if ih >= self.colwidths.len() {
                    self.colwidths.push(0);
                }
                self.colwidths[ih] = self.colwidths[ih].max(3 + line.width() as u16);
            }
            header_height = header_height.max(height);
            drawable_header.push(Cell::from(Text::from(s)));
        }
        let n_cols = self.colwidths.len() + 1;

        let mut column_widths = self.colwidths.iter().map(|i| Constraint::Length(*i)).collect::<Vec<_>>();
        let drawable_header = Row::new(drawable_header).height(header_height).underlined();
        let drawable_rows = rows
            .into_iter()
            .map(|r| {
                let row = r.to_row(data);
                let mut row_height = row.iter().map(|l| l.lines.len()).max().unwrap();
                let mut cells = vec![];
                for (ic, column) in row.into_iter().enumerate() {
                    assert!(n_cols > ic, "rows are not the same length");
                    let mut height = 0;
                    let mut s = Text::default();
                    for line in column.lines {
                        s.push_line(Line::from(format!("▏ {} ", line)));
                        height += 1;
                        if ic != n_cols - 1 {
                            self.colwidths[ic] = self.colwidths[ic].max(3 + line.width() as u16);
                        }
                    }
                    for _ in 0..row_height - height {
                        s.push_line(Line::from("▏"));
                    }
                    row_height = row_height.max(height);

                    cells.push(Cell::from(s.set_style(column.style)));
                }
                height_avail = height_avail.saturating_sub(row_height as u16);

                (Row::new(cells).height(row_height as u16), height_avail)
            })
            .take_while(|(_, height)| *height > 0)
            .map(|(row, _)| row);

        *column_widths.last_mut().unwrap() = Constraint::Fill(1);
        let table = Table::new(drawable_rows, column_widths)
            .header(drawable_header)
            .style(Style::new())
            .column_spacing(0)
            .row_highlight_style(Style::default().add_modifier(Modifier::REVERSED).fg(Color::DarkGray));

        frame.render_stateful_widget(table, area, &mut self.state)
    }
}
