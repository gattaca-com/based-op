mod collections;
mod data;
mod prelude;
mod statistics;
mod timekeeper;
mod types;
mod ui;
mod utils;

use std::io::stdout;

use bop_common::{
    communication::Consumer,
    telemetry::{TelemetryUpdate, system::SystemNotification, system_notificiations_queue, telemetry_queue},
    time::Timer,
    time::{Nanos, utils::renderloop_60_fps},
};
use crossterm::{
    ExecutableCommand,
    event::{self, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use data::{BuiltBlocksMode, Data};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style, Stylize, palette::tailwind},
    text::Line,
    widgets::{Block, Borders, Clear, Tabs},
};
use strum_macros::{Display, EnumIter, FromRepr};
use timekeeper::{TimeKeeper, TimeKeeperMode};
use tracing::warn;

#[derive(Copy, Clone, Debug, Default, Display, FromRepr, EnumIter)]
enum Mode {
    #[default]
    #[strum(to_string = "Overview")]
    Overview,
    #[strum(to_string = "Timing Realtime")]
    TimekeeperRealtime,
    #[strum(to_string = "Timing FlameGraph")]
    TimekeeperFlameGraph,
}

impl Mode {
    /// Get the previous tab, if there is no previous tab return the current tab.
    fn previous(self) -> Self {
        let current_index: usize = self as usize;
        let previous_index = current_index.saturating_sub(1);
        Self::from_repr(previous_index).unwrap_or(self)
    }

    /// Get the next tab, if there is no next tab return the current tab.
    fn next(self) -> Self {
        let current_index = self as usize;
        let next_index = current_index.saturating_add(1);
        Self::from_repr(next_index).unwrap_or(self)
    }

    /// Return tab's name as a styled `Line`
    fn title(self) -> Line<'static> {
        format!("  {self}  ").fg(tailwind::SLATE.c200).bg(self.palette().c900).into()
    }

    const fn palette(self) -> tailwind::Palette {
        match self {
            Self::Overview => tailwind::NEUTRAL,
            Self::TimekeeperRealtime => tailwind::FUCHSIA,
            Self::TimekeeperFlameGraph => tailwind::BLUE,
        }
    }
}

struct OverseerConsumers {
    telemetry: Consumer<TelemetryUpdate>,
    system_notifications: Consumer<SystemNotification>,
}

impl Default for OverseerConsumers {
    fn default() -> Self {
        Self { telemetry: telemetry_queue().into(), system_notifications: system_notificiations_queue().into() }
    }
}

struct OverseerTimers {
    total: Timer,
    render: Timer,
}
impl Default for OverseerTimers {
    fn default() -> Self {
        Self { total: Timer::new("Overseer"), render: Timer::new("Overseer-render") }
    }
}

const BLOCK_TSTAMP: Nanos = Nanos::from_secs(1729699211);
const SLOT_DURATION: Nanos = Nanos::from_secs(12);

#[derive(Default)]
struct Overseer {
    timers: OverseerTimers,
    data: Data,
    mode: Mode,
    search_string: Option<String>,
    prev_selected_slot: Option<u64>,
}

impl Overseer {
    pub fn update(&mut self, consumers: &mut OverseerConsumers, slot_time: bool) {
        self.data.new_slot = false;
        self.data.update(consumers, slot_time);
    }

    pub fn render(&mut self, frame: &mut Frame) {
        self.timers.render.start();
        use Constraint::{Length, Min};
        let vertical = Layout::vertical([Length(1), Min(0), Length(1)]);
        let [tabs_area, inner_area, footer_area] = vertical.areas(frame.area());

        self.render_tabs(tabs_area, frame);
        self.render_footer(footer_area, frame);

        let layout = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(22), Constraint::Fill(1)])
            .split(inner_area);
        match self.mode {
            Mode::Overview if !self.data.is_empty() => {
                self.data.strat_toggles_visible = false;
                SlotInfoSidebar {}.render(&mut self.data, frame, layout[0]);
                StrategiesOverview {}.render(&self.data, frame, layout[1]);
            }

            Mode::TimekeeperRealtime => {
                self.data.timekeeper.set_mode(TimeKeeperMode::RealTime);
                TimeKeeper::render(&mut self.data, inner_area, frame)
            }
            Mode::TimekeeperFlameGraph if self.data.last_slot != 0 => {
                self.data.timekeeper.set_mode(TimeKeeperMode::FlameGraph);
                TimeKeeper::render(&mut self.data, inner_area, frame)
            }
            _ => {}
        }

        if let Some(string) = &self.search_string {
            self.render_search(frame, string);
        }

        self.timers.render.stop();
    }

    fn next_tab(&mut self) {
        self.mode = self.mode.next();
    }

    fn previous_tab(&mut self) {
        self.mode = self.mode.previous();
    }

    fn render_tabs(&self, area: Rect, buf: &mut Frame) {
        let titles = Mode::iter().map(Mode::title);
        let highlight_style = (Color::default(), self.mode.palette().c700);
        let selected_tab_index = self.mode as usize;
        buf.render_widget(
            Tabs::new(titles).highlight_style(highlight_style).select(selected_tab_index).padding("", "").divider(" "),
            area,
        );
    }

    fn render_search(&self, frame: &mut Frame, string: &str) {
        let style = if string.parse::<u64>().ok().is_some_and(|parsed| self.data.has_slot_data(parsed)) {
            Style::default().fg(Color::Green)
        } else {
            Style::default().fg(Color::Red)
        };
        let modal_area = modal_area(frame.area(), 40, 4);
        let block = Block::new()
            .title("Enter slot number")
            .borders(Borders::all())
            .style(style)
            .title_bottom("<Enter>: select")
            .title_bottom(format!("<Esc>: slot {}", self.prev_selected_slot.unwrap()));
        let inner = block.inner(modal_area);
        frame.render_widget(Clear, modal_area);
        frame.render_widget(block, modal_area);
        let line = Line::styled(string, style);
        frame.render_widget(line, inner);
    }

    pub fn handle_key_events(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        match (code, modifiers, &self.mode) {
            (KeyCode::Right, _, _) => self.next_tab(),
            (KeyCode::Left, _, _) => self.previous_tab(),
            (KeyCode::Esc, _, Mode::Overview) if self.search_string.is_some() => {
                self.search_string = None;
                self.data.display_slot(self.prev_selected_slot.take().unwrap());
            }
            (KeyCode::Char(' '), _, Mode::Overview | Mode::TimekeeperRealtime | Mode::TimekeeperFlameGraph) => {
                self.data.display_last_slot()
            }
            (KeyCode::Up, _, Mode::Overview) => self.data.strat_toggle_list_state.select_previous(),
            (KeyCode::Down, _, Mode::Overview) => self.data.strat_toggle_list_state.select_next(),
            (KeyCode::Enter, _, Mode::Overview) if self.search_string.is_some() => {
                let Some(parsed) = self.search_string.as_ref().and_then(|s| s.parse::<u64>().ok()) else {
                    self.search_string = None;
                    return;
                };
                self.data.display_slot(parsed);
                self.search_string = None;
                self.prev_selected_slot = None;
            }
            (KeyCode::Enter, _, Mode::Overview) => {
                self.data.toggle_strat_visibility(modifiers.contains(KeyModifiers::ALT))
            }

            (KeyCode::Tab, _, Mode::Overview) => self.data.toggle_strat_toggles_visibility(),
            (KeyCode::Char('m'), _, Mode::TimekeeperRealtime) => {
                self.data.displayed_mut().time_datas.toggle_render_options(RenderFlags::ShowMin)
            }
            (KeyCode::Char('M'), _, Mode::TimekeeperRealtime) => {
                self.data.displayed_mut().time_datas.toggle_render_options(RenderFlags::ShowMax)
            }
            (KeyCode::Char('e'), _, Mode::TimekeeperRealtime) => {
                self.data.displayed_mut().time_datas.toggle_render_options(RenderFlags::ShowMedian)
            }
            (KeyCode::Char('t'), _, Mode::TimekeeperRealtime) => {
                self.data.displayed_mut().time_datas.toggle_render_options(RenderFlags::ShowTotal)
            }
            (KeyCode::Char('a'), _, Mode::TimekeeperRealtime) => {
                self.data.displayed_mut().time_datas.toggle_render_options(RenderFlags::ShowAverages)
            }
            (c, _, Mode::TimekeeperRealtime | Mode::TimekeeperFlameGraph) => self.data.timekeeper.handle_key_events(c),
            (KeyCode::Char(c), _, _) if self.search_string.is_some() => {
                let s = self.search_string.as_mut().unwrap();
                s.push(c);
                if let Some(parsed) = s.parse::<u64>().ok().filter(|parsed| self.data.has_slot_data(*parsed)) {
                    self.data.display_slot(parsed);
                }
            }
            (KeyCode::Backspace, _, _) if self.search_string.is_some() => {
                let s = self.search_string.as_mut().unwrap();
                s.pop();
                if let Some(parsed) = s.parse::<u64>().ok().filter(|parsed| self.data.has_slot_data(*parsed)) {
                    self.data.display_slot(parsed);
                }
            }
            (KeyCode::Char('f'), _, Mode::Overview) => {
                let curslot = self.data.displayed_slot();
                self.search_string = Some(curslot.to_string());
                self.prev_selected_slot = Some(curslot);
            }
            (KeyCode::Char('f'), _, Mode::BlocksBuilt)
                if matches!(self.data.built_blocks_tab.mode, BuiltBlocksMode::BlocksTable(_)) =>
            {
                let curslot = self.data.displayed_slot();
                self.search_string = Some(curslot.to_string());
                self.prev_selected_slot = Some(curslot);
            }
            (KeyCode::Char('f'), _, Mode::CexDex) if matches!(self.data.cex_dex_tab.mode, CexDexMode::Table(_)) => {
                let curslot = self.data.displayed_slot();
                self.search_string = Some(curslot.to_string());
                self.prev_selected_slot = Some(curslot);
            }
            _ => {}
        }
    }
    fn render_footer(&self, area: Rect, frame: &mut Frame) {
        let txt = match self.mode {
            Mode::Overview => "◄ ► change tab | ▲ ▼ to select | Enter toggle | Q to quit",
            Mode::TimekeeperRealtime => {
                "◄ ► change tab | ▲ ▼ to select | m: min, a: avg, e: med, M: max | l: latency, b: business | f/PageUp/Down/Space to change slot | Q to quit"
            }
            Mode::TimekeeperFlameGraph => "",
        };
        frame.render_widget(Line::raw(txt).centered(), area)
    }
}

fn modal_area(area: Rect, width_x: u16, width_y: u16) -> Rect {
    let vertical = Layout::vertical([Constraint::Length(width_y)]).flex(ratatui::layout::Flex::Center);
    let horizontal = Layout::horizontal([Constraint::Length(width_x)]).flex(ratatui::layout::Flex::Center);
    let [area] = vertical.areas(area);
    let [area] = horizontal.areas(area);
    area
}

fn main() {
    stdout().execute(EnterAlternateScreen).unwrap();
    enable_raw_mode().unwrap();
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout())).unwrap();
    let _ = terminal.clear();

    let enable_logging = std::env::var("ENABLE_LOGGING").map(|val| val.to_lowercase() == "true").unwrap_or(true);
    if enable_logging {
        let _guard = initialise_tracing_log("overseer.log", 100, None);
    }
    tracing::info!("Overseer starting");
    let mut consumers = OverseerConsumers::default();

    let mut overseer = Overseer::default();

    let mut cur_t_stamp = BLOCK_TSTAMP + ((Nanos::now() - BLOCK_TSTAMP) / SLOT_DURATION) * SLOT_DURATION;

    renderloop_60_fps(|| {
        overseer.timers.total.start();
        let slot_time = cur_t_stamp.elapsed() > SLOT_DURATION;
        if slot_time {
            cur_t_stamp += SLOT_DURATION;
        }
        overseer.update(&mut consumers, slot_time);
        if let Err(e) = terminal.draw(|frame| {
            overseer.render(frame);
        }) {
            warn!("issue drawing terminal {e}")
        }

        if !event::poll(std::time::Duration::ZERO).unwrap_or_default() {
            return true;
        }

        if let Ok(event::Event::Key(KeyEvent { kind: KeyEventKind::Press, code, modifiers, .. })) = event::read() {
            if let KeyCode::Char('Q') = &code {
                return false;
            }
            overseer.handle_key_events(code, modifiers);
        }
        overseer.timers.total.stop();
        true
    });
    stdout().execute(LeaveAlternateScreen).unwrap();
    disable_raw_mode().unwrap();
}
