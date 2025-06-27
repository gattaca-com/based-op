mod collections;
mod data;
mod prelude;
mod statistics;
mod timekeeper;
mod types;
mod ui;
mod utils;

use std::io::stdout;

use alloy_primitives::{Address, B256};
use bop_common::{
    api::{
        self, OpGethPeer, OpNodeApiClient, OpNodeP2PApiClient, OpPeerInfo, RegistryApiClient, RollupConfig, SyncStatus,
    },
    communication::{Consumer, WalkieTalkie},
    config::{LoggingConfig, LoggingFlags},
    signing::ECDSASigner,
    telemetry::{telemetry_queue, TelemetryUpdate},
    time::{utils::renderloop_60_fps, Nanos},
    utils::init_tracing,
};
use clap::Parser;
use crossterm::{
    event::{self, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use data::{Data, UIData};
use jsonrpsee::{core::ClientError, http_client::HttpClient};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Layout, Rect},
    style::{palette::tailwind, Color, Stylize},
    text::Line,
    widgets::Tabs,
    Frame, Terminal,
};
use reqwest::Url;
use strum::IntoEnumIterator;
use strum_macros::{Display, EnumIter, FromRepr};
use timekeeper::{TimeKeeper, TimeKeeperMode};
use tracing::warn;
use ui::plot::RenderFlags;

#[derive(Parser, Debug, Clone)]
#[command(version, about, name = "based-overseer")]
pub struct OverseerArgs {
    /// The url of the portal that is connected to the main sequencer node
    #[arg(short, long)]
    pub portal_url: Url,
    /// The url of the based-op-node running next to the based-gateway
    #[arg(long, default_value = "http://127.0.0.1:8547")]
    pub based_op_node_url: Url,
    /// The url of the based-op-geth running next to the based-gateway
    #[arg(long, default_value = "http://127.0.0.1:8645")]
    pub based_op_geth_url: Url,

    #[arg(long, default_value = None)]
    pub rich_wallet_key: Option<B256>,
    #[arg(long, default_value = "5")]
    pub max_tx_send_retries: usize,
}

#[derive(Copy, Clone, Debug, Default, Display, FromRepr, EnumIter)]
enum Mode {
    #[default]
    #[strum(to_string = "Overview")]
    Overview,
    #[strum(to_string = "Tx Spammer")]
    TxSpammer,
    #[strum(to_string = "Timing Realtime")]
    TimekeeperRealtime,
    #[strum(to_string = "Timing FlameGraph")]
    TimekeeperFlameGraph,
    #[strum(to_string = "General Chain Info")]
    ChainInfo,
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
            Self::TxSpammer => tailwind::RED,
            Self::TimekeeperRealtime => tailwind::FUCHSIA,
            Self::TimekeeperFlameGraph => tailwind::BLUE,
            Self::ChainInfo => tailwind::AMBER,
        }
    }
}

struct OverseerConnections {
    telemetry: Consumer<TelemetryUpdate>,
    walkie_talkie: WalkieTalkie,
    runtime: tokio::runtime::Runtime,
    client_portal: HttpClient,
    client_based_op_node: HttpClient,
    client_based_op_geth: HttpClient,
    rollup_config: RollupConfig,
}

impl OverseerConnections {
    pub fn new(args: &OverseerArgs) -> Self {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("Couldn't initialize tokio runtime");
        let client_portal =
            HttpClient::builder().build(args.portal_url.clone()).expect("Couldn't initialize portal rpc client");
        let client_based_op_node = HttpClient::builder()
            .build(args.based_op_node_url.clone())
            .expect("Couldn't initialize based-op-node rpc client");
        let client_based_op_geth = HttpClient::builder()
            .build(args.based_op_geth_url.clone())
            .expect("Couldn't initialize based-op-geth rpc client");

        let rollup_config: RollupConfig =
            runtime.block_on(client_portal.rollup_config()).expect("couldn't read rollup_config").into();

        Self {
            walkie_talkie: WalkieTalkie::default(),
            telemetry: telemetry_queue().into(),
            runtime,
            client_portal,
            client_based_op_node,
            client_based_op_geth,
            rollup_config,
        }
    }

    #[allow(dead_code)]
    pub fn rollup_config(&self) -> Result<RollupConfig, ClientError> {
        self.runtime.block_on(self.client_portal.rollup_config())
    }

    pub fn sync_status_global(&self) -> Result<SyncStatus, ClientError> {
        self.runtime.block_on(self.client_portal.sync_status())
    }

    pub fn sync_status_local(&self) -> Result<SyncStatus, ClientError> {
        self.runtime.block_on(self.client_based_op_node.sync_status())
    }

    pub fn current_gateway(&self) -> Result<(Url, Address), ClientError> {
        self.runtime.block_on(self.client_portal.current_gateway()).map(|(_, url, address, _)| (url, address))
    }

    pub fn peers_based_op_node(&self) -> Result<Vec<OpPeerInfo>, ClientError> {
        self.runtime.block_on(self.client_based_op_node.peers(true)).map(|p| p.peers.into_values().collect())
    }

    pub fn peers_based_op_geth(&self) -> Result<Vec<OpGethPeer>, ClientError> {
        self.runtime.block_on(api::OpGethAdminApiClient::peers(&self.client_based_op_geth))
    }
}

struct Overseer {
    data: Data,
    ui_data: UIData,
    mode: Mode,
    tx_spam_account: Option<(ECDSASigner, u64)>,
}
impl Overseer {
    pub fn new(args: &OverseerArgs, rollup_config: RollupConfig) -> Self {
        let uri_based_op_geth = args.based_op_geth_url.to_string().parse().expect("couldn't parse portal url");
        let rich_wallet_key = args.rich_wallet_key.and_then(|k| {
            ECDSASigner::try_from_secret(k.as_slice())
                .inspect_err(|e| tracing::warn!("Couldn't generated ECDSASigner from rich wallet key: {e}"))
                .ok()
        });
        Self {
            ui_data: Default::default(),
            data: Data::new(rollup_config.clone(), uri_based_op_geth, rich_wallet_key, args.max_tx_send_retries),
            mode: Default::default(),
            tx_spam_account: None,
        }
    }

    pub fn block_duration(&self) -> Nanos {
        Nanos::from_secs(self.data.rollup_config.block_time)
    }

    pub fn genesis_time(&self) -> Nanos {
        Nanos::from_secs(self.data.rollup_config.genesis.l2_time)
    }

    pub fn update(&mut self, consumers: &mut OverseerConnections, slot_time: bool) {
        match &mut self.tx_spam_account {
            Some((account, nonce)) => {
                *nonce = self.data.maybe_spam_more_txs(&mut consumers.walkie_talkie, account, *nonce);
            }
            None => {
                self.tx_spam_account = self.data.airdrop_eth(&mut consumers.walkie_talkie).map(|account| (account, 0))
            }
        }
        self.data.update(consumers, slot_time);
    }

    pub fn render(&mut self, frame: &mut Frame) {
        if self.data.is_empty() {
            return;
        }
        use Constraint::{Length, Min};
        let vertical = Layout::vertical([Length(1), Min(0), Length(1)]);
        let [tabs_area, inner_area, footer_area] = vertical.areas(frame.area());

        self.render_tabs(tabs_area, frame);
        self.render_footer(footer_area, frame);

        match self.mode {
            Mode::Overview => {
                self.ui_data.render_overview(&self.data, inner_area, frame);
            }

            Mode::TxSpammer => {
                self.ui_data.render_tx_spammer(&self.data, inner_area, frame);
            }

            Mode::TimekeeperRealtime => {
                self.data.timekeeper.set_mode(TimeKeeperMode::RealTime);
                TimeKeeper::render(&mut self.data, inner_area, frame)
            }
            Mode::TimekeeperFlameGraph => {
                self.data.timekeeper.set_mode(TimeKeeperMode::FlameGraph);
                TimeKeeper::render(&mut self.data, inner_area, frame)
            }
            Mode::ChainInfo => {
                self.ui_data.render_chain_info(&self.data, inner_area, frame.buffer_mut());
            }
        }
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

    pub fn handle_key_events(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        match (code, modifiers, &self.mode) {
            (KeyCode::Right, _, _) => self.next_tab(),
            (KeyCode::Left, _, _) => self.previous_tab(),
            (KeyCode::Up | KeyCode::Char('k'), _, Mode::ChainInfo) => self.ui_data.scroll_view_state.scroll_up(),
            (KeyCode::Down | KeyCode::Char('j'), _, Mode::ChainInfo) => self.ui_data.scroll_view_state.scroll_down(),
            (KeyCode::PageUp, _, Mode::ChainInfo) => self.ui_data.scroll_view_state.scroll_page_up(),
            (KeyCode::PageDown, _, Mode::ChainInfo) => self.ui_data.scroll_view_state.scroll_page_down(),
            (KeyCode::Char('-'), _, Mode::Overview | Mode::ChainInfo | Mode::TxSpammer) => {
                self.data.tx_spammer.tps = self.data.tx_spammer.tps.saturating_sub(1).max(1);
            }
            (KeyCode::Char('+'), _, Mode::Overview | Mode::ChainInfo | Mode::TxSpammer) => {
                self.data.tx_spammer.tps = self.data.tx_spammer.tps.saturating_add(1);
            }
            (KeyCode::Char('m'), _, Mode::TimekeeperRealtime) => {
                self.data.time_datas.toggle_render_options(RenderFlags::ShowMin)
            }
            (KeyCode::Char('M'), _, Mode::TimekeeperRealtime) => {
                self.data.time_datas.toggle_render_options(RenderFlags::ShowMax)
            }
            (KeyCode::Char('e'), _, Mode::TimekeeperRealtime) => {
                self.data.time_datas.toggle_render_options(RenderFlags::ShowMedian)
            }
            (KeyCode::Char('t'), _, Mode::TimekeeperRealtime) => {
                self.data.time_datas.toggle_render_options(RenderFlags::ShowTotal)
            }
            (KeyCode::Char('a'), _, Mode::TimekeeperRealtime) => {
                self.data.time_datas.toggle_render_options(RenderFlags::ShowAverages)
            }
            (c, _, Mode::TimekeeperRealtime | Mode::TimekeeperFlameGraph) => self.data.timekeeper.handle_key_events(c),
            _ => {}
        }
    }

    fn render_footer(&self, area: Rect, frame: &mut Frame) {
        let txt = match self.mode {
            Mode::Overview | Mode::ChainInfo | Mode::TxSpammer => "◄ ► change tab | +- increase/decrease tps | Q to quit",
            Mode::TimekeeperRealtime => {
                "◄ ► change tab | ▲ ▼ to select | m: min, a: avg, e: med, M: max | l: latency, b: business | f/PageUp/Down/Space to change slot | Q to quit"
            }
            Mode::TimekeeperFlameGraph => "",
        };
        frame.render_widget(Line::raw(txt).centered(), area)
    }
}

fn main() {
    stdout().execute(EnterAlternateScreen).unwrap();
    enable_raw_mode().unwrap();
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout())).unwrap();
    let _ = terminal.clear();
    let mut logging_config = LoggingConfig::default_with_prefix("overseer.log".to_string());
    logging_config.flags = LoggingFlags::File;
    let _guard = init_tracing(logging_config);
    tracing::info!("Overseer starting");
    let args = OverseerArgs::parse();
    let mut consumers = OverseerConnections::new(&args);
    let mut overseer: Overseer = Overseer::new(&args, consumers.rollup_config.clone());
    let genesis_time = overseer.genesis_time();
    let block_duration = overseer.block_duration();

    let mut cur_t_stamp = genesis_time + ((Nanos::now() - genesis_time) / block_duration) * block_duration;

    renderloop_60_fps(|| {
        let slot_time = cur_t_stamp.elapsed() > block_duration;
        if slot_time {
            cur_t_stamp += block_duration;
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
        true
    });
    stdout().execute(LeaveAlternateScreen).unwrap();
    disable_raw_mode().unwrap();
}
