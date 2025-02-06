use std::io::stdout;

use bop_common::{
    communication::queues_dir_string,
    time::{utils::renderloop_60_fps, Nanos},
};
use crossterm::{
    event::{self, KeyCode, KeyEvent, KeyEventKind},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Layout},
    text::Line,
    Terminal,
};
use bop_timekeeper::TimeKeeper;
use tracing::warn;

const BLOCK_TSTAMP: Nanos = Nanos::from_secs(1729699211);

fn main() {
    stdout().execute(EnterAlternateScreen).unwrap();
    enable_raw_mode().unwrap();
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout())).unwrap();
    let _ = terminal.clear();
    let mut timekeeper = TimeKeeper::new(128, 256, queues_dir_string());

    let mut cur_t_stamp = BLOCK_TSTAMP + ((Nanos::now() - BLOCK_TSTAMP) / Nanos::from_secs(2)) * Nanos::from_secs(2);
    renderloop_60_fps(|| {
        let block_start = cur_t_stamp.elapsed() > Nanos::from_secs(2);
        if block_start {
            cur_t_stamp += Nanos::from_secs(2)
        }
        timekeeper.update(block_start);
        if let Err(e) = terminal.draw(|frame| {
            let vertical = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]);
            let [main_area, footer_area] = vertical.areas(frame.area());

            let footer = "▲ ▼ to select Timer | Toggles: m = min, a = avg, e = med, M = max, l = latency, b = business | +,- to change interval per datapoint | Space to pause | q to quit";
            frame.render_widget(Line::raw(footer).centered(), footer_area);
            timekeeper.render(main_area, frame);
        }) {
            warn!("issue drawing terminal {e}")
        }

        if !event::poll(std::time::Duration::ZERO).unwrap_or_default() {
            return true;
        }

        if let Ok(event::Event::Key(KeyEvent { kind: KeyEventKind::Press, code, .. })) = event::read() {
            if let KeyCode::Char('q') = &code {
                return false;
            }
            timekeeper.handle_key_events(code);
        }
        true
    });
    stdout().execute(LeaveAlternateScreen).unwrap();
    disable_raw_mode().unwrap();
}
