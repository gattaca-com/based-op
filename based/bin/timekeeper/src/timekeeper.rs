use std::fmt::Debug;

use bop_common::{communication::{Consumer, Queue}, time::{Duration, Instant, Repeater, TimingMessage}};
use crossterm::event::KeyCode;
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, HighlightSpacing, List, ListItem, ListState, Paragraph},
};

use crate::{statistics::Statistics, tui::RenderFlags};

bitflags::bitflags! {
    #[repr(transparent)]
    #[derive(Clone, Copy, Debug, PartialEq, PartialOrd, Ord, Eq, Default)]
    pub struct PaneFlags: u8 {
        const ShowLatency  = 0b00000001;
        const ShowBusiness = 0b00000010;
    }
}

#[derive(Clone, Debug)]
struct TimerData {
    name: String,
    latency_data: Statistics<Duration>,
    latency_consumer: Consumer<TimingMessage>,
    processing_data: Statistics<Duration>,
    processing_consumer: Consumer<TimingMessage>,
    direction: Direction,
    flags: PaneFlags,
}

impl TimerData {
    pub fn new(
        name: String,
        samples_per_median: usize,
        n_datapoints: usize,
        clock_overhead: Duration,
        latency_consumer: Consumer<TimingMessage>,
        processing_consumer: Consumer<TimingMessage>,
    ) -> Self {
        Self {
            name,
            latency_data: Statistics::new("Latency".into(), samples_per_median, n_datapoints, clock_overhead),
            latency_consumer,
            processing_data: Statistics::new("Business".into(), samples_per_median, n_datapoints, clock_overhead),
            processing_consumer,
            direction: Direction::Vertical,
            flags: PaneFlags::ShowBusiness | PaneFlags::ShowLatency,
        }
    }

    pub fn register_datapoint(&mut self, block_start: bool) {
        self.latency_data.register_datapoint(self.latency_consumer.tot_published(), block_start);
        self.processing_data.register_datapoint(self.processing_consumer.tot_published(), block_start);
    }

    pub fn handle_messages(&mut self) {
        self.latency_data.handle_messages(&mut self.latency_consumer);
        self.processing_data.handle_messages(&mut self.processing_consumer);
    }

    pub fn report(&self, frame: &mut Frame, rect: Rect) {
        match (
            self.latency_data.is_empty() || !self.flags.contains(PaneFlags::ShowLatency),
            self.processing_data.is_empty() || !self.flags.contains(PaneFlags::ShowBusiness),
        ) {
            (true, true) => frame.render_widget(Paragraph::new("No timing data to display"), rect),
            (false, true) => {
                let layout = Layout::new(self.direction, [Constraint::Percentage(80), Constraint::Percentage(20)]).split(rect);
                self.latency_data.report(&self.name, frame, layout[0]);
                self.latency_data.report_msg_per_sec(frame, layout[1]);
            }
            (true, false) => {
                let layout = Layout::new(self.direction, [Constraint::Percentage(80), Constraint::Percentage(20)]).split(rect);
                self.processing_data.report(&self.name, frame, layout[0]);
                self.processing_data.report_msg_per_sec(frame, layout[1])
            }
            (false, false) => {
                let layout =
                    Layout::new(self.direction, [Constraint::Percentage(40), Constraint::Percentage(40), Constraint::Percentage(20)]).split(rect);
                self.latency_data.report(&self.name, frame, layout[0]);
                self.processing_data.report(&self.name, frame, layout[1]);
                self.processing_data.report_msg_per_sec(frame, layout[2]);
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.latency_data.is_empty() && self.processing_data.is_empty()
    }

    pub fn toggle_render_options(&mut self, flags: RenderFlags) {
        self.latency_data.toggle(flags);
        self.processing_data.toggle(flags);
    }

    pub fn toggle_pane_options(&mut self, flags: PaneFlags) {
        self.flags ^= flags;
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

#[derive(Clone, Debug, Default)]
struct TimeDatas {
    data: Vec<TimerData>,
    state: ListState,
}

impl TimeDatas {
    pub fn push(&mut self, data: TimerData) {
        self.data.push(data);
        self.data.sort_unstable_by(|t1, t2| t1.name.cmp(&t2.name))
    }
    pub fn contains(&self, name: &str) -> bool {
        self.data.iter().any(|d| d.name == name)
    }

    pub fn toggle_render_options(&mut self, options: RenderFlags) {
        self.data.iter_mut().for_each(|d| d.toggle_render_options(options))
    }

    pub fn toggle_pane_options(&mut self, options: PaneFlags) {
        self.data.iter_mut().for_each(|d| d.toggle_pane_options(options))
    }

    pub fn select_next(&mut self) {
        self.state.select_next()
    }

    pub fn select_previous(&mut self) {
        self.state.select_previous()
    }

    pub fn selected(&self) -> Option<usize> {
        self.state.selected()
    }
}

pub struct TimeKeeper {
    samples_per_median: usize,
    n_datapoints: usize,
    clock_overhead: Duration,
    queue_checker: Repeater,
    data_gatherer: Repeater,
    paused: bool,
    time_datas: TimeDatas,
    queues_dir: String,
}

impl TimeKeeper {
    pub fn new(samples_per_median: usize, n_datapoints: usize, queues_dir: String) -> Self {
        let clock_overhead = clock_overhead();
        let data_gatherer = Repeater::every(Duration::from_millis(100));
        let queue_checker = Repeater::every(Duration::from_secs(10));
        Self {
            samples_per_median,
            n_datapoints,
            clock_overhead,
            queue_checker,
            data_gatherer,
            paused: false,
            time_datas: Default::default(),
            queues_dir,
        }
    }

    fn check_new_queues(&mut self) {
        let queues_dir = &self.queues_dir;
        let Ok(entries) = std::fs::read_dir(queues_dir) else {
            return;
        };
        for entry in entries.filter_map(|e| e.ok()) {
            let name = entry.path().as_os_str().to_str().unwrap().to_string();
            if name.contains("latency") {
                let (_dir, real_name) = name.split_once("latency-").unwrap();

                if !self.time_datas.contains(real_name) {
                    let latency_q = Queue::open_shared(format!("{queues_dir}/latency-{real_name}")).expect("couldn't open latency queue");
                    let processing_q = Queue::open_shared(format!("{queues_dir}/timing-{real_name}")).expect("couldn't open timing queue");
                    let d = TimerData::new(
                        real_name.to_string().clone(),
                        self.samples_per_median,
                        self.n_datapoints,
                        self.clock_overhead,
                        latency_q.into(),
                        processing_q.into(),
                    );
                    self.time_datas.push(d);
                }
            }
        }
    }

    pub fn render(&mut self, area: Rect, frame: &mut Frame) {
        if self.paused {
            return;
        }
        let layout =
            Layout::default().direction(Direction::Horizontal).constraints([Constraint::Percentage(20), Constraint::Percentage(80)]).split(area);

        let mut time_datas = self.time_datas.data.iter().filter(|d| !d.is_empty());

        let namelist: Vec<ListItem> = time_datas.clone().map(|data| ListItem::from(data.name.clone())).collect();

        let block = Block::new().title("Timers").borders(Borders::ALL);
        let selected_style = Style::default().bold();
        let list = List::new(namelist)
            .block(block)
            .highlight_style(selected_style)
            .highlight_symbol(">")
            .highlight_spacing(HighlightSpacing::Always)
            .scroll_padding(2);

        frame.render_stateful_widget(list, layout[0], &mut self.time_datas.state);
        if let Some(time_data) = self.time_datas.selected().and_then(|curid| time_datas.nth(curid)) {
            time_data.report(frame, layout[1]);
        }
    }

    pub fn update(&mut self, block_start: bool) {
        if self.queue_checker.fired() {
            self.check_new_queues();
        };
        for d in &mut self.time_datas.data {
            d.handle_messages();
        }
        if self.data_gatherer.fired() || block_start {
            self.time_datas.data.iter_mut().for_each(|t| t.register_datapoint(block_start));
        }
    }

    pub fn handle_key_events(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char('+') => self.data_gatherer += Duration::from_millis(20),
            KeyCode::Char('-') => self.data_gatherer -= Duration::from_millis(20),
            KeyCode::Char('m') => self.time_datas.toggle_render_options(RenderFlags::ShowMin),
            KeyCode::Char('M') => self.time_datas.toggle_render_options(RenderFlags::ShowMax),
            KeyCode::Char('e') => self.time_datas.toggle_render_options(RenderFlags::ShowMedian),
            KeyCode::Char('a') => self.time_datas.toggle_render_options(RenderFlags::ShowAverages),
            KeyCode::Char('b') => self.time_datas.toggle_pane_options(PaneFlags::ShowBusiness),
            KeyCode::Char('l') => self.time_datas.toggle_pane_options(PaneFlags::ShowLatency),
            KeyCode::Char(' ') => self.paused = !self.paused,
            KeyCode::Down => self.time_datas.select_next(),
            KeyCode::Up => self.time_datas.select_previous(),
            //TODO: Add modal help list of commands
            _ => {}
        }
    }
}
