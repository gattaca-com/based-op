use std::fmt::Write;

use ratatui::{
    style::{Modifier, Styled},
    text::Line,
    widgets::{Row, TableState},
};

use crate::{
    prelude::*,
    types::{Ords, PerMillage},
};
pub mod block;
pub mod built_blocks;
pub mod order;
pub mod plot;

#[derive(Debug, Clone)]
pub struct CyclingListState {
    state: ListState,
    padding: usize,
    n: usize,
}

impl Default for CyclingListState {
    fn default() -> Self {
        Self { state: Default::default(), padding: 2, n: 0 }
    }
}

impl CyclingListState {
    pub fn select_next(&mut self) {
        if let Some(selected) = self.state.selected() {
            if selected == self.n.saturating_sub(1) {
                self.state.select(Some(0));
                return;
            }
        }
        self.state.select_next();
    }

    pub fn select_previous(&mut self) {
        if let Some(selected) = self.state.selected() {
            if selected == 0 {
                self.state.select(Some(self.n));
                return;
            }
        }
        self.state.select_previous();
    }

    pub fn render<'a, T>(&mut self, items: T, frame: &mut Frame, area: Rect)
    where
        T: IntoIterator,
        T::Item: Into<ListItem<'a>>,
    {
        if self.selected().is_none() {
            self.select_first();
        }
        let selected_style = Style::default().bold();
        let list = List::new(items)
            .highlight_style(selected_style)
            .highlight_symbol(">")
            .highlight_spacing(ratatui::widgets::HighlightSpacing::Always)
            .scroll_padding(self.padding);
        self.n = list.len();
        frame.render_stateful_widget(list, area, &mut self.state)
    }
}

impl Deref for CyclingListState {
    type Target = ListState;

    fn deref(&self) -> &Self::Target {
        &self.state
    }
}

impl DerefMut for CyclingListState {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.state
    }
}

#[derive(Debug, Clone, Default)]
pub struct CyclingTableState<K> {
    state: TableState,
    keys: Vec<K>,
    colwidths: Vec<u16>,
    prev_selected: Option<K>,
}

impl<K: Clone + PartialEq> CyclingTableState<K> {
    pub fn with_prev_slected(mut self, prev_selected: K) -> Self {
        self.prev_selected = Some(prev_selected);
        self
    }

    pub fn select_next(&mut self) {
        if let Some(selected) = self.state.selected() {
            if selected == self.keys.len().saturating_sub(1) {
                self.state.select(Some(0));
                return;
            }
        }
        self.state.select_next();
    }

    pub fn select_previous(&mut self) {
        if let Some(selected) = self.state.selected() {
            if selected == 0 {
                self.state.select(Some(self.keys.len()));
                return;
            }
        }
        self.state.select_previous();
    }

    pub fn selected(&self) -> Option<K> {
        self.state.selected().map(|i| self.keys[i].clone())
    }

    pub fn render<'a, 'b, S: Into<Text<'b>>>(
        &mut self,
        title: Option<String>,
        header: impl Iterator<Item = S>,
        rows: impl Iterator<Item = (K, Vec<Text<'a>>)>,
        frame: &mut Frame,
        area_: Rect,
    ) {
        self.keys.clear();
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
        let drawable_header = Row::new(drawable_header).height(header_height).underlined();
        let mut drawable_rows = vec![];
        for (i, (key, row)) in rows.into_iter().enumerate() {
            self.keys.push(key.clone());
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
            let mut row = Row::new(cells).height(row_height as u16);
            if self.prev_selected.as_ref().is_some_and(|k| key == *k) {
                row = row.style(Style::default().underlined().underline_color(Color::Red).bold());
                if self.state.selected().is_none() {
                    self.state.select(Some(i))
                }
            }
            drawable_rows.push(row);
        }

        if self.state.selected().is_none() && self.keys.len() > 1 {
            self.select_first();
        }

        let mut column_widths = self.colwidths.iter().map(|i| Constraint::Length(*i)).collect::<Vec<_>>();
        *column_widths.last_mut().unwrap() = Constraint::Fill(1);
        let table = Table::new(drawable_rows, column_widths)
            .header(drawable_header)
            .style(Style::new())
            .column_spacing(0)
            .row_highlight_style(Style::default().add_modifier(Modifier::REVERSED).fg(Color::DarkGray));

        frame.render_stateful_widget(table, area, &mut self.state)
    }
}

impl<K> Deref for CyclingTableState<K> {
    type Target = TableState;

    fn deref(&self) -> &Self::Target {
        &self.state
    }
}
impl<K> DerefMut for CyclingTableState<K> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.state
    }
}

