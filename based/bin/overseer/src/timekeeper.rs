use std::{collections::HashMap, fmt::Debug};

use bop_common::{
    communication::Consumer,
    time::{Duration, Instant, TimingMessage},
};
use crossterm::event::KeyCode;
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, HighlightSpacing, LineGauge, List, ListItem, ListState, Paragraph},
};
use strum_macros::AsRefStr;
use style::palette::tailwind;

use crate::data::{Data, TimerData};

bitflags::bitflags! {
    #[repr(transparent)]
    #[derive(Clone, Copy, Debug, PartialEq, PartialOrd, Ord, Eq, Default)]
    pub struct PaneFlags: u8 {
        const ShowLatency  = 0b00000001;
        const ShowBusiness = 0b00000010;
    }
}

fn black_box<T>(dummy: T) -> T {
    unsafe { std::ptr::read_volatile(&dummy) }
}

pub fn clock_overhead() -> Duration {
    let start = Instant::now();
    for _ in 0..1_000_000 {
        black_box(Instant::now());
    }
    let end = Instant::now();
    Duration((end.0 - start.0) / 1_000_000)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, AsRefStr, Default)]
#[repr(u8)]
pub enum TimeKeeperMode {
    #[default]
    RealTime,
    FlameGraph,
}

#[derive(Clone, Debug, Default)]
pub struct TimeKeeper {
    paused: bool,
    mode: TimeKeeperMode,
    pub timers_list_state: ListState,
    pub timers: HashMap<String, TimerDataState>,
}

impl TimeKeeper {
    fn render_realtime(data: &mut Data, area: Rect, frame: &mut Frame) {
        if data.timekeeper.paused {
            return;
        }

        let slot_time_data = data.time_datas.data.iter().filter(|d| !d.is_empty());

        let mut max = 0;
        let namelist: Vec<ListItem> = slot_time_data
            .clone()
            .map(|data| {
                let name = data.name.clone();
                max = max.max(name.len());
                ListItem::from(name)
            })
            .collect();

        let layout = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(max as u16 + 4), Constraint::Fill(1)])
            .split(area);

        let block = Block::new().title(format!("Block {}", data.block_number)).borders(Borders::ALL);
        let selected_style = Style::default().bold();
        let list = List::new(namelist)
            .block(block)
            .highlight_style(selected_style)
            .highlight_symbol(">")
            .highlight_spacing(HighlightSpacing::Always)
            .scroll_padding(2);

        frame.render_stateful_widget(list, layout[0], &mut data.timekeeper.timers_list_state);
        if let Some(time_data) =
            data.timekeeper.timers_list_state.selected().and_then(|curid| slot_time_data.clone().nth(curid))
        {
            let timer = data.timekeeper.timers.get(&time_data.name).unwrap();
            timer.report(time_data, frame, layout[1]);
        }
    }

    fn render_flamegraph(data: &Data, area: Rect, frame: &mut Frame<'_>) {
        let b = Block::new().title(format!("Block {}", data.block_number)).borders(Borders::all());
        frame.render_widget(b.clone(), area);
        let area = b.inner(area);

        let time_datas = data.time_datas.data.iter().filter(|d| !d.is_empty());
        let n_time_datas = time_datas.clone().count();

        let layout = Layout::vertical(vec![Constraint::Length(1); n_time_datas]);
        let areas = layout.split(area);

        let Some(max_t) = time_datas.clone().map(|t| t.tot_processing).max() else {
            return;
        };
        let width1 = time_datas.clone().map(|t| t.name.len()).max().unwrap() + 2;
        let width2 = time_datas.clone().map(|t| t.tot_processing.to_string().len()).max().unwrap();

        for (td, area) in time_datas.zip(areas.iter()) {
            let label = format!("{:width1$} {:width2$}", format!("{}: ", td.name), td.tot_processing.to_string());
            let ratio = if max_t.0 == 0 { 1 } else { td.tot_processing.0 * 100 / max_t.0 };
            frame.render_widget(
                LineGauge::default().filled_style(tailwind::BLUE.c800).ratio(ratio as f64 / 100.0).label(label),
                *area,
            );
        }
    }

    pub fn render(data: &mut Data, area: Rect, frame: &mut Frame) {
        match data.timekeeper.mode {
            TimeKeeperMode::RealTime => Self::render_realtime(data, area, frame),
            TimeKeeperMode::FlameGraph => Self::render_flamegraph(data, area, frame),
        }
    }

    pub fn handle_key_events(&mut self, code: KeyCode) {
        match (self.mode, code) {
            (TimeKeeperMode::RealTime, KeyCode::Char(' ')) => self.paused = !self.paused,
            (TimeKeeperMode::RealTime, KeyCode::Down) => self.timers_list_state.select_next(),
            (TimeKeeperMode::RealTime, KeyCode::Up) => self.timers_list_state.select_previous(),
            (TimeKeeperMode::RealTime, KeyCode::Char('b')) => self.toggle_pane_options(PaneFlags::ShowBusiness),
            (TimeKeeperMode::RealTime, KeyCode::Char('l')) => self.toggle_pane_options(PaneFlags::ShowLatency),
            _ => {}
        }
    }

    pub fn set_mode(&mut self, mode: TimeKeeperMode) {
        self.mode = mode
    }

    pub fn toggle_pane_options(&mut self, options: PaneFlags) {
        self.timers.values_mut().for_each(|d| d.toggle_pane_options(options))
    }
}

#[derive(Clone, Debug)]
pub struct TimerDataState {
    pub latency_consumer: Consumer<TimingMessage>,
    pub processing_consumer: Consumer<TimingMessage>,
    pub direction: Direction,
    pub flags: PaneFlags,
}

impl TimerDataState {
    pub fn new(latency_consumer: Consumer<TimingMessage>, processing_consumer: Consumer<TimingMessage>) -> Self {
        Self {
            latency_consumer,
            processing_consumer,
            direction: Direction::Vertical,
            flags: PaneFlags::ShowBusiness | PaneFlags::ShowLatency,
        }
    }

    pub fn report(&self, data: &TimerData, frame: &mut Frame, rect: Rect) {
        match (
            data.latency_data.is_empty() || !self.flags.contains(PaneFlags::ShowLatency),
            data.processing_data.is_empty() || !self.flags.contains(PaneFlags::ShowBusiness),
        ) {
            (true, true) => frame.render_widget(Paragraph::new("No timing data to display"), rect),
            (false, true) => {
                let layout =
                    Layout::new(self.direction, [Constraint::Percentage(80), Constraint::Percentage(20)]).split(rect);
                data.latency_data.report(&data.name, frame, layout[0]);
                data.latency_data.report_msg_per_sec(frame, layout[1]);
            }
            (true, false) => {
                let layout =
                    Layout::new(self.direction, [Constraint::Percentage(80), Constraint::Percentage(20)]).split(rect);
                data.processing_data.report(&data.name, frame, layout[0]);
                data.processing_data.report_msg_per_sec(frame, layout[1])
            }
            (false, false) => {
                let layout = Layout::new(self.direction, [
                    Constraint::Percentage(40),
                    Constraint::Percentage(40),
                    Constraint::Percentage(20),
                ])
                .split(rect);
                data.latency_data.report(&data.name, frame, layout[0]);
                data.processing_data.report(&data.name, frame, layout[1]);
                data.processing_data.report_msg_per_sec(frame, layout[2]);
            }
        }
    }

    pub fn toggle_pane_options(&mut self, flags: PaneFlags) {
        self.flags ^= flags;
    }
}
