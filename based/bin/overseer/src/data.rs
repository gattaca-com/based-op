use std::{collections::VecDeque, io::Write};

use alloy_primitives::{Address, B256, U256};
use block::BlockData;
use bop_common::{
    api::{OpGethPeer, OpPeerInfo, RollupConfig, SyncStatus},
    communication::{
        queues_dir,
        walkie_talkie::{self, RpcResponse},
        Consumer, Queue, WalkieTalkie,
    },
    signing::ECDSASigner,
    telemetry::{frag::Frag, order::Tx, system::SystemNotification, Telemetry},
    time::{Duration, TimingMessage},
    typedefs::HashMap,
};
use frag::FragData;
use http::{Request, Uri};
use mio_httpc::{CallRef, Response};
use op_alloy_rpc_types::OpTransactionReceipt;
use ratatui::{buffer::Buffer, layout::Size, widgets::Wrap};
use reqwest::Url;
use system::SystemNotificationData;
use transaction::{SpamData, SpammedTx, TransactionData};
use tui_scrollview::ScrollViewState;

use crate::{
    collections::{CircularBuffer, KeyedCircularBuffer},
    prelude::*,
    statistics::Statistics,
    timekeeper::{clock_overhead, TimeKeeper, TimerDataState},
    ui::plot::RenderFlags,
    utils::empty_if_default,
    OverseerConnections,
};

pub mod block;
pub mod frag;
pub mod system;
pub mod transaction;

#[derive(Copy, Clone, Debug)]
pub struct PendingTransaction {
    sent_timestamp: Nanos,
    nonce: u64,
    retries: usize,
    from_address: Address,
    tx_hash: Option<B256>,
}

#[derive(Clone, Debug, Default)]
pub struct TxSpammer {
    // transfers sent for which we're waiting for receipt replies
    pending_transfers: HashMap<CallRef, PendingTransaction>,
    pending_retries: VecDeque<PendingTransaction>,
    max_retries: usize,
    uri_based_op_geth: Uri,
    rich_wallet_key: Option<ECDSASigner>,
    chain_id: u64,
    pub tps: usize,
    tx_resetter: Repeater,
    n_txs_sent: usize,
}

impl TxSpammer {
    pub fn new(
        max_retries: usize,
        uri_based_op_geth: Uri,
        rich_wallet_key: Option<ECDSASigner>,
        chain_id: u64,
    ) -> Self {
        Self {
            max_retries,
            uri_based_op_geth,
            rich_wallet_key,
            chain_id,
            tps: 50,
            tx_resetter: Repeater::every(Duration::from_secs(1)),
            ..Default::default()
        }
    }

    fn airdrop_eth(&self, walkie_talkie: &mut WalkieTalkie) -> Option<ECDSASigner> {
        self.rich_wallet_key.as_ref().and_then(|signer| {
            let new_signer = ECDSASigner::random();
            let to_account = new_signer.address;
            let value = U256::from(1_000_000_000_000_000u64);
            if let Some(_callref) = signer.send_transfer(
                walkie_talkie,
                self.uri_based_op_geth.clone(),
                self.chain_id,
                to_account,
                value,
                None,
            ) {
                let mut resp_buf = Vec::new();
                loop {
                    walkie_talkie.gather_responses(&mut resp_buf);
                    break;
                }
            } else {
                tracing::warn!("airdrop: failed");
            };
            Some(new_signer)
        })
    }

    fn send_transfer_to_self(
        &mut self,
        walkie_talkie: &mut WalkieTalkie,
        signer: &ECDSASigner,
        nonce: u64,
    ) -> Option<SpammedTx> {
        let value = U256::from(1u64);
        if let Some(callref) = signer.send_transfer(
            walkie_talkie,
            self.uri_based_op_geth.clone(),
            self.chain_id,
            signer.address,
            value,
            Some(nonce),
        ) {
            let sent_timestamp = Nanos::now();
            self.pending_transfers.insert(callref, PendingTransaction {
                sent_timestamp,
                retries: 0,
                from_address: signer.address,
                tx_hash: None,
                nonce,
            });

            Some(SpammedTx {
                wallet: signer.address,
                sent_timestamp,
                nonce,
                hash: None,
                block: None,
                receipt_timestamp: None,
            })
        } else {
            None
        }
    }

    pub fn maybe_resend_later(&mut self, mut pending: PendingTransaction) {
        if pending.retries < self.max_retries {
            pending.retries += 1;
            self.pending_retries.push_back(pending);
        } else {
            tracing::warn!("{pending:?} failed after 5 reqs, keep retrying");
            pending.retries = 0;
            self.pending_retries.push_back(pending);
        }
    }

    pub fn send_receipt_request(&mut self, walkie_talkie: &mut WalkieTalkie, pending: PendingTransaction) {
        let payload = serde_json::to_vec(&serde_json::json!({
            "id": 1,
            "jsonrpc": "2.0",
            "method": "eth_getTransactionReceipt",
            "params": [pending.tx_hash.unwrap()]
        }))
        .unwrap();
        let Ok(req) = Request::builder()
            .header("content-type", "application/json")
            .method("POST")
            .uri(self.uri_based_op_geth.clone())
            .body(payload)
        else {
            tracing::warn!("couldn't create request");
            self.maybe_resend_later(pending);
            return;
        };
        let Ok(callref) = walkie_talkie.send(req).inspect_err(|e| {
            tracing::warn!("issue sending nonce request to portal: {e}");
        }) else {
            self.maybe_resend_later(pending);
            return;
        };
        self.pending_transfers.insert(callref, pending);
    }

    fn has_callref(&self, callref: &CallRef) -> bool {
        self.pending_transfers.contains_key(callref)
    }

    fn is_status_ok(
        &mut self,
        resp: Response,
        body: &Vec<u8>,
        pending: PendingTransaction,
    ) -> Option<PendingTransaction> {
        if resp.status != 200 {
            tracing::warn!(
                "got response status {} for {pending:?}: {resp:?} - {}",
                resp.status,
                std::str::from_utf8(body).unwrap()
            );
            self.maybe_resend_later(pending);
            return None;
        }
        Some(pending)
    }

    fn handle_response(
        &mut self,
        walkie_talkie: &mut WalkieTalkie,
        resp: walkie_talkie::Result<(CallRef, Response, Vec<u8>)>,
        f: impl FnOnce(SpammedTx),
    ) {
        let Ok((callref, res, body)) = resp.inspect_err(|e| {
            tracing::warn!("got an issue when receiving a transaction response {:?}: {e}", e.callref());
            if let Some(pending) = self.pending_transfers.remove(e.callref()) {
                self.maybe_resend_later(pending);
            }
        }) else {
            return;
        };

        let Some(pending) = self.pending_transfers.remove(&callref) else {
            tracing::warn!("got transaction response for a callref I don't know about");
            return;
        };
        let Some(mut pending) = self.is_status_ok(res, &body, pending) else {
            return;
        };

        match &pending.tx_hash {
            Some(_hash) => {
                let Some(receipt) = serde_json::from_slice::<RpcResponse<Option<OpTransactionReceipt>>>(&body).ok()
                else {
                    tracing::warn!("issue parsing receipt for {pending:?}: {}", String::from_utf8(body).unwrap());
                    self.maybe_resend_later(pending);
                    return;
                };
                let Some(receipt) = receipt.result.flatten() else {
                    self.send_receipt_request(walkie_talkie, pending);
                    return;
                };

                f(SpammedTx::new(
                    pending.sent_timestamp,
                    pending.from_address,
                    pending.nonce,
                    Some(receipt.inner.transaction_hash),
                    Some(receipt.inner.block_number.unwrap_or_default()),
                    Some(Nanos::now()),
                ));
            }
            None => {
                let Some(hash) = serde_json::from_slice::<RpcResponse<B256>>(&body)
                    .inspect_err(|e| {
                        tracing::warn!(
                            "couldn't parse eth_sendRawTransaction response from {}: {e}",
                            String::from_utf8(body).unwrap_or_default()
                        );
                    })
                    .ok()
                    .and_then(|t| t.result)
                else {
                    self.maybe_resend_later(pending);
                    return;
                };
                pending.tx_hash = Some(hash);
                self.send_receipt_request(walkie_talkie, pending);
            }
        }
    }

    pub fn maybe_spam_more_txs(
        &mut self,
        walkie_talkie: &mut WalkieTalkie,
        signer: &ECDSASigner,
        mut nonce: u64,
        mut f: impl FnMut(SpammedTx),
    ) -> u64 {
        if self.tx_resetter.fired() {
            self.n_txs_sent = 0
        }

        let tps = self.tps as f64;
        let tx_per_frame = tps / 60.0;
        let tx_this_frame =
            (self.tx_resetter.elapsed().as_secs() * tps - self.n_txs_sent as f64 + tx_per_frame).ceil() as usize;

        for _ in 0..tx_this_frame.min(self.tps.saturating_sub(self.pending_transfers.len())) {
            if let Some(tx) = self.send_transfer_to_self(walkie_talkie, signer, nonce) {
                self.n_txs_sent += 1;
                f(tx);
                nonce += 1;
            } else {
                break;
            }
        }
        nonce
    }
}

#[derive(Clone, Debug, Default)]
pub struct UIData {
    pub table_blocks: TableState,
    pub table_frags: TableState,
    pub table_pool: TableState,
    pub table_system: TableState,
    pub table_spam: TableState,
    pub scroll_view_state: ScrollViewState,
}

impl UIData {
    pub fn render_overview(&mut self, data: &Data, area: Rect, frame: &mut Frame) {
        let [top, bottom] = Layout::vertical([Constraint::Length(12), Constraint::Fill(1)]).areas(area);
        let [top_left, top_middle] = Layout::horizontal([Constraint::Percentage(35), Constraint::Fill(1)]).areas(top);

        self.render_system_overview(data, top_left, frame);
        self.render_sync_status(data, top_middle, frame);

        let [left, middle, right] =
            Layout::horizontal([Constraint::Percentage(35), Constraint::Percentage(35), Constraint::Fill(1)])
                .areas(bottom);

        let [middle_top, middle_bottom] = Layout::vertical([Constraint::Length(8), Constraint::Fill(1)]).areas(middle);

        self.render_keybindings_and_branding(data, middle_top, frame);

        self.render_system_messages(data, right, frame);

        let [left, bottom_left] = Layout::vertical([Constraint::Percentage(50), Constraint::Fill(1)]).areas(left);
        self.table_blocks.render(
            Some("Blocks".to_string()),
            BlockData::header(),
            data.sealed_blocks(),
            data,
            frame,
            left,
        );
        self.table_frags.render(
            Some(format!("Frags in currently sequenced Block {}", data.block_number)),
            FragData::block_table_header(),
            data.current_block_frags(),
            data,
            frame,
            bottom_left,
        );
        self.table_pool.render(
            Some("Tx Pool".to_string()),
            TransactionData::pool_header(),
            data.transactions.iter().rev(),
            data,
            frame,
            middle_bottom,
        )
    }

    pub fn render_chain_info(&mut self, data: &Data, area: Rect, buf: &mut Buffer) {
        let rollup_config = format!("{:#?}", data.rollup_config);
        let rollup_len = rollup_config.lines().count() as u16 + 2;
        let rollup_paragraph = Paragraph::new(rollup_config).block(Block::new().borders(Borders::all()));

        let peers = format!("{:#?}", data.peers_local_op_node);
        let peers_len = peers.lines().count() as u16 + 2;
        let peers_paragraph = Paragraph::new(peers)
            .wrap(Wrap { trim: false })
            .block(Block::new().title("Op Node Peers").borders(Borders::all()));

        let peers_geth = format!("{:#?}", data.peers_local_op_geth);
        let peers_len_geth = peers_geth.lines().count() as u16 + 2;
        let peers_paragraph_geth = Paragraph::new(peers_geth)
            .wrap(Wrap { trim: false })
            .block(Block::new().title("Op Geth Peers").borders(Borders::all()));

        let width = if area.height < peers_len + rollup_len + peers_len_geth { area.width - 1 } else { area.width };

        let mut scroll_view =
            tui_scrollview::ScrollView::new(Size::new(width, rollup_len + peers_len + peers_len_geth));
        let [top, mid, bottom] =
            Layout::vertical([Constraint::Length(rollup_len), Constraint::Length(peers_len), Constraint::Fill(1)])
                .areas(scroll_view.area());

        scroll_view.render_widget(rollup_paragraph, top);
        scroll_view.render_widget(peers_paragraph, mid);
        scroll_view.render_widget(peers_paragraph_geth, bottom);

        <tui_scrollview::ScrollView as ratatui::widgets::StatefulWidget>::render(
            scroll_view,
            area,
            buf,
            &mut self.scroll_view_state,
        );
    }

    fn render_keybindings_and_branding(&mut self, _data: &Data, area: Rect, frame: &mut Frame) {
        let mut tw = tabwriter::TabWriter::new(vec![]);
        let _ = writeln!(&mut tw, "Q:\tQuit");
        let _ = writeln!(&mut tw, "◄ ►:\tChange Tab");

        tw.flush().unwrap();
        let txt = String::from_utf8(tw.into_inner().unwrap()).unwrap();
        let info = Paragraph::new(txt)
            .block(Block::new().title("Key Bindings").borders(Borders::all()).fg(Color::from_u32(0x800080)));
        let [left, right] = Layout::horizontal([Constraint::Length(20), Constraint::Fill(1)]).areas(area);
        let big_text = tui_big_text::BigText::builder()
            .pixel_size(tui_big_text::PixelSize::Full)
            .style(Style::new().blue())
            .centered()
            .lines(vec!["Based Op".fg(Color::from_u32(0x800080)).into()])
            .build();
        frame.render_widget(big_text, right);
        frame.render_widget(info, left);
    }

    fn render_system_overview(&mut self, data: &Data, area: Rect, frame: &mut Frame) {
        let mut tw = tabwriter::TabWriter::new(vec![]);
        let _ = writeln!(
            &mut tw,
            "Last Update:\t{}",
            data.system.last().map(|t| t.timestamp.with_fmt("%d %H:%M:%S%.3f")).unwrap_or_default()
        );

        let SystemNotificationData { timestamp: t, notification: cur_state } = data.current_state().unwrap_or_default();
        if let Some((url, address)) = data.current_gateway.as_ref() {
            let _ = writeln!(&mut tw, "Current Gateway:\t{url}");
            let _ = writeln!(&mut tw, "Signing wallet:\t{address}");
        } else {
            let _ = writeln!(&mut tw, "Current Gateway:");
            let _ = writeln!(&mut tw);
        }

        let _ = writeln!(&mut tw, "Current Block:\t{}", data.block_number);
        let _ = writeln!(&mut tw, "Current State:\t{}", cur_state.as_ref());
        let SystemNotificationData { notification: last_state, .. } = data.last_state().unwrap_or_default();
        let _ = writeln!(&mut tw, "Prev State:\t{}", empty_if_default(last_state.as_ref()));
        let _ = writeln!(&mut tw, "Last State Transition:\t{}", empty_if_default(t));
        let local_op_bn = data.sync_status_local.1.unsafe_l2.number;
        if local_op_bn == 0 {
            let _ = writeln!(&mut tw, "Based Op Node sync:\tUnreachable/Is Down");
        } else {
            let _ = writeln!(&mut tw, "Based Op Node sync:\t{local_op_bn}");
        }
        let _ = writeln!(&mut tw, "Based Op Node peers:\t{}", data.peers_local_op_node.len());
        let _ = writeln!(&mut tw, "Based Op Geth peers:\t{}", data.peers_local_op_geth.len());

        tw.flush().unwrap();
        let txt = String::from_utf8(tw.into_inner().unwrap()).unwrap();
        let info = Paragraph::new(txt).block(Block::new().title("Based-Gateway Status").borders(Borders::all()));

        frame.render_widget(info, area);
    }

    fn render_sync_status(&mut self, data: &Data, area: Rect, frame: &mut Frame) {
        let mut tw = tabwriter::TabWriter::new(vec![]);
        let _ = writeln!(&mut tw, "Last Update:\t{}", data.sync_status_global.0.with_fmt("%d %H:%M:%S%.3f"));

        let _ = writeln!(
            &mut tw,
            "Unsafe L2:\t{}\t{}",
            data.sync_status_global.1.unsafe_l2.number, data.sync_status_global.1.unsafe_l2.hash
        );
        let _ = writeln!(
            &mut tw,
            "Safe L2:\t{}\t{}",
            data.sync_status_global.1.safe_l2.number, data.sync_status_global.1.safe_l2.hash
        );
        let _ = writeln!(
            &mut tw,
            "Finalized L2:\t{}\t{}",
            data.sync_status_global.1.finalized_l2.number, data.sync_status_global.1.finalized_l2.hash
        );
        let _ = writeln!(
            &mut tw,
            "Pending Safe L2:\t{}\t{}",
            data.sync_status_global.1.pending_safe_l2.number, data.sync_status_global.1.pending_safe_l2.hash
        );
        let _ = writeln!(
            &mut tw,
            "L1:\t{}\t{}",
            data.sync_status_global.1.current_l1.number, data.sync_status_global.1.current_l1.hash
        );
        let _ = writeln!(
            &mut tw,
            "Safe L1:\t{}\t{}",
            data.sync_status_global.1.safe_l1.number, data.sync_status_global.1.safe_l1.hash
        );
        let _ = writeln!(
            &mut tw,
            "Finalized L1:\t{}\t{}",
            data.sync_status_global.1.current_l1_finalized.number, data.sync_status_global.1.current_l1_finalized.hash
        );
        let _ = writeln!(
            &mut tw,
            "Head L1:\t{}\t{}",
            data.sync_status_global.1.head_l1.number, data.sync_status_global.1.head_l1.hash
        );

        tw.flush().unwrap();
        let txt = String::from_utf8(tw.into_inner().unwrap()).unwrap();
        let info = Paragraph::new(txt).block(Block::new().title("Global Chain Status").borders(Borders::all()));

        frame.render_widget(info, area);
    }

    fn render_system_messages(&mut self, data: &Data, right: Rect, frame: &mut Frame<'_>) {
        self.table_system.render(
            Some("System Messages".to_string()),
            vec![Text::from("Timestamp"), Text::from("Message")].into_iter(),
            data.system.iter().rev(),
            data,
            frame,
            right,
        );
    }

    pub fn render_tx_spammer(&mut self, data: &Data, inner_area: Rect, frame: &mut Frame<'_>) {
        let [left, right] =
            Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(inner_area);
        let [left_top, left_bottom] = Layout::vertical([Constraint::Length(6), Constraint::Fill(1)]).areas(left);

        let mut tw = tabwriter::TabWriter::new(vec![]);
        let _ = writeln!(
            &mut tw,
            "Wallet:\t{}",
            data.tx_spam_account.as_ref().map(|(address, _)| address.address.to_string()).unwrap_or_else(|| {
                if data.tx_spammer.rich_wallet_key.is_some() {
                    "Hit <enter> to start spamming".to_string()
                } else {
                    "Start Overseer with --rich-wallet-key to start spamming".to_string()
                }
            })
        );

        let _ = writeln!(
            &mut tw,
            "Nonce:\t{}",
            data.spammed_txs.iter().last().map(|t| t.nonce.to_string()).unwrap_or_default()
        );

        let _ = writeln!(&mut tw, "Tps:\t{} (+- to increase/decrease)", data.tx_spammer.tps.to_string());

        let (min, avg, med, max) = data.spam_data.latencies();
        let _ = writeln!(&mut tw, "Latency:\tAvg: {avg:>11}\tMin: {min:>11}\tMax: {max:>11}\tMed: {med:>11}");

        tw.flush().unwrap();
        let txt = String::from_utf8(tw.into_inner().unwrap()).unwrap();
        let info = Paragraph::new(txt).block(Block::new().title("Spam Stats").borders(Borders::all()));
        frame.render_widget(info, left_top);
        data.spam_data.report(frame, right);

        self.table_spam.render(
            Some("".to_string()),
            SpammedTx::header(),
            data.spammed_txs.iter().rev(),
            data,
            frame,
            left_bottom,
        );
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TimerData {
    pub name: String,
    pub latency_data: Statistics<Duration>,
    pub processing_data: Statistics<Duration>,
    pub tot_processing: Duration,
}

impl TimerData {
    pub fn new(name: String, samples_per_median: usize, n_datapoints: usize, clock_overhead: Duration) -> Self {
        Self {
            name,
            latency_data: Statistics::new("Latency".into(), samples_per_median, n_datapoints, clock_overhead),
            processing_data: Statistics::new("Business".into(), samples_per_median, n_datapoints, clock_overhead),
            tot_processing: Duration::ZERO,
        }
    }

    pub fn handle_messages(
        &mut self,
        latency_consumer: &mut Consumer<TimingMessage>,
        processing_consumer: &mut Consumer<TimingMessage>,
    ) {
        self.latency_data.handle_messages(latency_consumer);
        self.processing_data.handle_messages(processing_consumer);
    }

    pub fn register_datapoint(
        &mut self,
        block_start: bool,
        increment_tot: bool,
        num_latencies_published: usize,
        num_processing_published: usize,
    ) {
        if increment_tot {
            self.tot_processing += Duration(self.processing_data.tot());
        }
        self.latency_data.register_datapoint(num_latencies_published, block_start);
        self.processing_data.register_datapoint(num_processing_published, block_start);
    }

    pub fn reset(&mut self) {
        self.tot_processing = Duration::ZERO;
    }

    pub fn is_empty(&self) -> bool {
        self.latency_data.is_empty() && self.processing_data.is_empty()
    }

    pub fn toggle_render_options(&mut self, flags: RenderFlags) {
        self.latency_data.toggle(flags);
        self.processing_data.toggle(flags);
    }
}

pub struct Data {
    pub block_number: u64,
    pub transactions: KeyedCircularBuffer<TransactionData>,
    pub blocks: KeyedCircularBuffer<BlockData>,
    pub frags: KeyedCircularBuffer<FragData>,
    pub system: CircularBuffer<SystemNotificationData>,
    pub data_gatherer: Repeater,
    pub queue_checker: Repeater,
    pub syncstatus_poller: Repeater,
    pub flamegraph_resetter: Repeater,
    pub timekeeper: TimeKeeper,
    pub time_datas: TimeDatas,
    pub rollup_config: RollupConfig,
    pub sync_status_global: (Nanos, SyncStatus),
    pub current_gateway: Option<(Url, Address)>,
    pub sync_status_local: (Nanos, SyncStatus),
    pub peers_local_op_node: Vec<OpPeerInfo>,
    pub peers_local_op_geth: Vec<OpGethPeer>,

    tx_spam_account: Option<(ECDSASigner, u64)>,
    pub spamming: bool,
    pub spammed_txs: KeyedCircularBuffer<SpammedTx>,
    pub tx_spammer: TxSpammer,
    pub spam_data: SpamData,
    results_buf: Vec<walkie_talkie::Result<(CallRef, Response, Vec<u8>)>>,
}
impl Data {
    const NUM_DATAPOINTS: usize = 256;
    const SAMPLES_PER_MEDIAN: usize = 128;

    pub fn new(
        rollup_config: RollupConfig,
        uri_based_op_geth: Uri,
        rich_wallet_key: Option<ECDSASigner>,
        max_tx_send_retries: usize,
    ) -> Self {
        Self {
            block_number: Default::default(),
            transactions: KeyedCircularBuffer::new(10_000),
            frags: KeyedCircularBuffer::new(10_000),
            blocks: KeyedCircularBuffer::new(10_000),
            system: CircularBuffer::new(10_000),
            spammed_txs: KeyedCircularBuffer::new(10_000),
            timekeeper: Default::default(),
            time_datas: Default::default(),
            data_gatherer: Repeater::every(Duration::from_secs(6) / 256u64),
            queue_checker: Repeater::every(Duration::from_secs(10)),
            syncstatus_poller: Repeater::every(Duration::from_secs(1)),
            flamegraph_resetter: Repeater::every(Duration::from_secs(60)),
            tx_spammer: TxSpammer::new(
                max_tx_send_retries,
                uri_based_op_geth,
                rich_wallet_key,
                rollup_config.l2_chain_id,
            ),
            rollup_config,
            sync_status_global: Default::default(),
            current_gateway: Default::default(),
            sync_status_local: Default::default(),
            peers_local_op_node: Default::default(),
            peers_local_op_geth: Default::default(),
            results_buf: Default::default(),
            spam_data: Default::default(),
            tx_spam_account: None,
            spamming: false,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.transactions.is_empty() && self.blocks.is_empty() && self.frags.is_empty()
    }

    pub fn insert_frag(&mut self, t: Nanos, frag: Uuid, update: Frag) {
        if let Frag::SorterStart { block, .. } = update {
            self.block_number = block;
        }
        if !self.blocks.contains_key(&self.block_number) {
            self.blocks.insert(BlockData::new(self.block_number, true, t));
        }

        let add_to_block = matches!(update, Frag::Commit);

        let frag = if !self.frags.contains_key(&frag) {
            self.frags.insert(FragData::new(t, frag, update));
            self.frags.get_mut(&frag).unwrap()
        } else {
            let f = self.frags.get_mut(&frag).unwrap();
            f.push(t, update);
            f
        };

        if add_to_block {
            if let Some((payment, gas_used, n_txs)) = frag.frag_stats() {
                self.blocks.get_mut(&self.block_number).unwrap().push(frag.uuid, payment, gas_used, n_txs);
            }
        }
    }

    /// Inserts bundle and Optionally returns block of update
    fn insert_transaction(&mut self, uuid: Uuid, t: Nanos, update: Tx) {
        if let Tx::Included(included_in_frag) = &update {
            if let Some(frag) = self.frags.get_mut(&included_in_frag.frag) {
                frag.txs.push(uuid)
            } else {
                tracing::warn!("weird, got an included message for a block we don't know")
            }
        }

        if let Some(data) = self.transactions.get_mut(&uuid) {
            data.push(t, update);
            return;
        }
        if matches!(update, Tx::RemovedFromPool) {
            return;
        }

        self.transactions.insert(TransactionData::new(uuid, t, update));
    }

    pub fn gather_pending_txs(&mut self, walkie_talkie: &mut WalkieTalkie) {
        walkie_talkie.gather_responses(&mut self.results_buf);
        for res in self.results_buf.drain(..) {
            let callref = match &res {
                Ok((r, _, _)) => r,
                Err(e) => e.callref(),
            };

            if self.tx_spammer.has_callref(callref) {
                self.tx_spammer.handle_response(walkie_talkie, res, |tx| {
                    self.spam_data.track(tx.latency());
                    self.spammed_txs.insert(tx);
                });
            }
        }
    }

    pub fn update(&mut self, consumers: &mut OverseerConnections, block_time: bool) {
        if self.spamming {
            if let Some((account, nonce)) = &mut self.tx_spam_account {
                *nonce = self.tx_spammer.maybe_spam_more_txs(&mut consumers.walkie_talkie, account, *nonce, |tx| {
                    self.spammed_txs.insert(tx);
                });
            }
        }
        self.gather_pending_txs(&mut consumers.walkie_talkie);

        while let Some(update) = consumers.telemetry.try_consume() {
            let (key, t, update) = update.into();
            match update {
                Telemetry::Tx(tx_update) => {
                    if let Tx::Included(included) = &tx_update {
                        if let Some(frag) = self.frags.get_mut(&included.frag) {
                            frag.add_tx(key, *included);
                        }
                    }
                    self.insert_transaction(key, t, tx_update);
                }
                Telemetry::Frag(update) => {
                    self.insert_frag(t, key, update);
                }
                Telemetry::System(system @ SystemNotification::BuildStop(curblock)) => {
                    self.block_number = curblock;
                    if let Some(block) = self.blocks.get_mut(&curblock) {
                        block.sealed = true;
                    }
                    self.system.push(SystemNotificationData::new(t, system));
                }
                Telemetry::System(system @ SystemNotification::BlockSync(block_number, gas_used)) => {
                    if !self.blocks.contains_key(&block_number) {
                        let mut block = BlockData::new(block_number, false, Nanos::now());
                        block.gas_used = gas_used;
                        self.blocks.insert(block);
                    } else {
                        let block = self.blocks.get_mut(&block_number).unwrap();
                        // This happens because we got an fcu with a different block than ours at some point and are now
                        // resyncing
                        if !block.sealed {
                            block.reset();
                        }
                    }
                    self.system.push(SystemNotificationData::new(t, system));
                    self.block_number = block_number;
                }
                Telemetry::System(system @ SystemNotification::NewPayload(block_number)) => {
                    if !self.blocks.contains_key(&block_number) {
                        let block = BlockData::new(block_number, false, Nanos::now());
                        self.blocks.insert(block);
                    }
                    self.block_number = block_number;
                    self.system.push(SystemNotificationData::new(t, system));
                }
                Telemetry::System(system @ SystemNotification::Sorting(block_number)) => {
                    if !self.blocks.contains_key(&block_number) {
                        let block = BlockData::new(block_number, true, Nanos::now());
                        self.blocks.insert(block);
                    }
                    self.block_number = block_number;
                    self.system.push(SystemNotificationData::new(t, system));
                }
                Telemetry::System(system) => {
                    self.system.push(SystemNotificationData::new(t, system));
                }
            }
        }
        if self.data_gatherer.fired() || block_time {
            for timer_data in self.time_datas.data.iter_mut() {
                if let Some(view) = self.timekeeper.timers.get_mut(&timer_data.name) {
                    timer_data.register_datapoint(
                        block_time,
                        true,
                        view.latency_consumer.tot_published(),
                        view.processing_consumer.tot_published(),
                    );
                    timer_data.handle_messages(&mut view.latency_consumer, &mut view.processing_consumer);
                }
            }
        }

        if self.queue_checker.fired() {
            self.check_new_queues();
        }
        if self.flamegraph_resetter.fired() {
            self.time_datas.reset()
        }
        if self.syncstatus_poller.fired() {
            if let Ok(sync_status_global) = consumers
                .sync_status_global()
                .inspect_err(|e| tracing::warn!("couldn't get SyncStatus from portal: {e}"))
            {
                self.sync_status_global = (Nanos::now(), sync_status_global)
            }

            if let Ok(sync_status_local) = consumers
                .sync_status_local()
                .inspect_err(|e| tracing::warn!("couldn't get SyncStatus from local based-op-node: {e}"))
            {
                self.sync_status_local = (Nanos::now(), sync_status_local)
            }

            self.current_gateway = consumers
                .current_gateway()
                .inspect_err(|e| tracing::warn!("couldn't get current sequencing based-gateway from portal: {e}"))
                .ok();

            if let Ok(mut peers) = consumers
                .peers_based_op_node()
                .inspect_err(|e| tracing::warn!("couldn't get peers of local based-op-node: {e}"))
            {
                peers.sort_unstable_by(|d1, d2| d1.peer_id.cmp(&d2.peer_id));
                self.peers_local_op_node = peers;
            }
            if let Ok(mut peers) = consumers
                .peers_based_op_geth()
                .inspect_err(|e| tracing::warn!("couldn't get peers of local based-op-geth: {e}"))
            {
                peers.sort_unstable_by(|d1, d2| d1.id.cmp(&d2.id));
                self.peers_local_op_geth = peers;
            }
        }
    }

    fn check_new_queues(&mut self) {
        let queues_dir = queues_dir();
        let Ok(entries) = std::fs::read_dir(&queues_dir) else {
            return;
        };
        for entry in entries.filter_map(|e| e.ok()) {
            let name = entry.path().as_os_str().to_str().unwrap().to_string();
            if name.contains("latency") {
                let (_dir, real_name) = name.split_once("latency-").unwrap();

                if !self.time_datas.contains(real_name) {
                    let latency_q = Queue::open_shared(queues_dir.join(format!("latency-{real_name}")))
                        .expect("couldn't open latency queue");
                    let processing_q = Queue::open_shared(queues_dir.join(format!("timing-{real_name}")))
                        .expect("couldn't open timing queue");
                    let view = TimerDataState::new(Consumer::new(latency_q, false), Consumer::new(processing_q, false));
                    let data = TimerData::new(
                        real_name.to_string().clone(),
                        Self::SAMPLES_PER_MEDIAN,
                        Self::NUM_DATAPOINTS,
                        clock_overhead(),
                    );

                    self.timekeeper.timers.insert(data.name.clone(), view);
                    self.time_datas.push(data);
                }
            }
        }
    }

    fn current_block_frags(&self) -> impl Iterator<Item = &FragData> {
        if self.blocks.get(&self.block_number).is_some_and(|b| b.sealed) {
            return either::Either::Left(std::iter::empty());
        }
        either::Either::Right(
            self.frags.iter().rev().take_while(|f| f.block_number().is_some_and(|n| n == self.block_number)),
        )
    }

    fn sealed_blocks(&self) -> impl Iterator<Item = &BlockData> {
        self.blocks.iter().rev().filter(|f| f.sealed)
    }

    fn current_state(&self) -> Option<SystemNotificationData> {
        self.system.iter().rev().find_map(|t| {
            if let SystemNotification::StateChanged(_) = t.notification {
                Some(*t)
            } else {
                None
            }
        })
    }

    fn last_state(&self) -> Option<SystemNotificationData> {
        self.system
            .iter()
            .rev()
            .filter_map(|t| if let SystemNotification::StateChanged(_) = t.notification { Some(*t) } else { None })
            .nth(1)
    }

    pub fn toggle_spamming(&mut self, walkie_talkie: &mut WalkieTalkie) {
        if self.tx_spam_account.is_none() {
            self.tx_spam_account = self.tx_spammer.airdrop_eth(walkie_talkie).map(|s| (s, 0));
        }
        self.spamming = !self.spamming;
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct TimeDatas {
    pub data: Vec<TimerData>,
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

    pub fn reset(&mut self) {
        self.data.iter_mut().for_each(|t| t.reset())
    }
}
