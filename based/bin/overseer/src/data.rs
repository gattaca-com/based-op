use std::io::Write;

use crate::{
    collections::{CircularBuffer, KeyedCircularBuffer},
    statistics::Statistics,
    ui::plot::RenderFlags,
    utils::empty_if_default,
};
use block::BlockData;
use bop_common::{
    api::{RollupConfig, SyncStatus},
    communication::{Consumer, Queue, queues_dir},
    telemetry::{
        Telemetry,
        frag::Frag,
        order::Tx,
        system::{SequencerState, SystemNotification},
    },
    time::{Duration, TimingMessage},
};
use frag::FragData;
use transaction::TransactionData;

use crate::{
    OverseerConsumers,
    prelude::*,
    timekeeper::{TimeKeeper, TimerDataState, clock_overhead},
};

pub mod block;
pub mod frag;
pub mod transaction;

#[derive(Clone, Debug, Default)]
pub struct UIData {
    pub table_blocks: TableState,
    pub table_frags: TableState,
    pub table_pool: TableState,
    pub table_system: TableState,
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

        self.render_system_messages(data, right, frame);

        let [left, bottom_left] = Layout::vertical([Constraint::Percentage(50), Constraint::Fill(1)]).areas(left);
        self.table_blocks.render(
            Some("Blocks".to_string()),
            BlockData::header(),
            data.sealed_blocks().map(|b| b.to_row()),
            frame,
            left,
        );
        self.table_frags.render(
            Some(format!("Frags in currently sequenced Block {}", data.block_number)),
            FragData::block_table_header(),
            data.current_block_frags().map(|b| b.to_block_table_row()),
            frame,
            bottom_left,
        );
        self.table_pool.render(
            Some("Tx Pool".to_string()),
            TransactionData::pool_header(),
            data.transactions.iter().map(|b| b.to_pool_row()).rev(),
            frame,
            middle,
        )
    }

    pub fn render_chain_info(&mut self, data: &Data, area: Rect, frame: &mut Frame) {
        frame.render_widget(Paragraph::new(format!("{:#?}", data.rollup_config)), area);
    }

    fn render_system_overview(&mut self, data: &Data, area: Rect, frame: &mut Frame) {
        let mut tw = tabwriter::TabWriter::new(vec![]);
        let _ = writeln!(
            &mut tw,
            "Last Update:\t{}",
            data.system.last().map(|(t, _)| t.with_fmt("%d %H:%M:%S%.3f")).unwrap_or_default()
        );

        let (t, cur_state) = data.current_state().unwrap_or_default();

        let _ = writeln!(&mut tw, "Current Block:\t{}", data.block_number);
        let _ = writeln!(&mut tw, "Current State:\t{}", cur_state.as_ref());
        let (_, last_state) = data.last_state().unwrap_or_default();
        let _ = writeln!(&mut tw, "Prev State:\t{}", empty_if_default(last_state));
        let _ = writeln!(&mut tw, "Last State Transition:\t{}", empty_if_default(t));
        tw.flush().unwrap();
        let txt = String::from_utf8(tw.into_inner().unwrap()).unwrap();
        let info = Paragraph::new(txt).block(Block::new().title("Local Gateway Status").borders(Borders::all()));

        frame.render_widget(info, area);
    }

    fn render_sync_status(&mut self, data: &Data, area: Rect, frame: &mut Frame) {
        let mut tw = tabwriter::TabWriter::new(vec![]);
        let _ = writeln!(&mut tw, "Last Update:\t{}", data.sync_status.0.with_fmt("%d %H:%M:%S%.3f"));

        let _ = writeln!(
            &mut tw,
            "Unsafe L2:\t{}\t{}",
            data.sync_status.1.unsafe_l2.number, data.sync_status.1.unsafe_l2.hash
        );
        let _ =
            writeln!(&mut tw, "Safe L2:\t{}\t{}", data.sync_status.1.safe_l2.number, data.sync_status.1.safe_l2.hash);
        let _ = writeln!(
            &mut tw,
            "Finalized L2:\t{}\t{}",
            data.sync_status.1.finalized_l2.number, data.sync_status.1.finalized_l2.hash
        );
        let _ = writeln!(
            &mut tw,
            "Pending Safe L2:\t{}\t{}",
            data.sync_status.1.pending_safe_l2.number, data.sync_status.1.pending_safe_l2.hash
        );
        let _ = writeln!(
            &mut tw,
            "Queued Un Safe L2:\t{}\t{}",
            empty_if_default(data.sync_status.1.queued_unsafe_l2.as_ref().map(|t| t.number).unwrap_or_default()),
            empty_if_default(data.sync_status.1.queued_unsafe_l2.as_ref().map(|t| t.hash).unwrap_or_default())
        );
        let _ =
            writeln!(&mut tw, "L1:\t{}\t{}", data.sync_status.1.current_l1.number, data.sync_status.1.current_l1.hash);
        let _ =
            writeln!(&mut tw, "Safe L1:\t{}\t{}", data.sync_status.1.safe_l1.number, data.sync_status.1.safe_l1.hash);
        let _ = writeln!(
            &mut tw,
            "Finalized L1:\t{}\t{}",
            data.sync_status.1.current_l1_finalized.number, data.sync_status.1.current_l1_finalized.hash
        );
        let _ =
            writeln!(&mut tw, "Head L1:\t{}\t{}", data.sync_status.1.head_l1.number, data.sync_status.1.head_l1.hash);
        tw.flush().unwrap();
        let txt = String::from_utf8(tw.into_inner().unwrap()).unwrap();
        let info = Paragraph::new(txt).block(Block::new().title("Chain Sync Status").borders(Borders::all()));

        frame.render_widget(info, area);
    }

    fn render_system_messages(&mut self, data: &Data, right: Rect, frame: &mut Frame<'_>) {
        self.table_system.render(
            Some("System Messages".to_string()),
            vec![Text::from("Timestamp"), Text::from("Message")].into_iter(),
            data.system
                .iter()
                .rev()
                .map(|(t, b)| vec![t.with_fmt("%d %H:%M:%S%.3f").into(), format!("{:?}", b).into()]),
            frame,
            right,
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
    pub system: CircularBuffer<(Nanos, SystemNotification)>,
    pub data_gatherer: Repeater,
    pub queue_checker: Repeater,
    pub syncstatus_poller: Repeater,
    pub flamegraph_resetter: Repeater,
    pub timekeeper: TimeKeeper,
    pub time_datas: TimeDatas,
    pub rollup_config: RollupConfig,
    pub sync_status: (Nanos, SyncStatus),
}
impl Data {
    pub fn new(rollup_config: RollupConfig) -> Self {
        Self {
            block_number: Default::default(),
            transactions: KeyedCircularBuffer::new(10_000),
            frags: KeyedCircularBuffer::new(10_000),
            blocks: KeyedCircularBuffer::new(10_000),
            system: CircularBuffer::new(10_000),
            timekeeper: Default::default(),
            time_datas: Default::default(),
            data_gatherer: Repeater::every(Duration::from_secs(6) / 256u64),
            queue_checker: Repeater::every(Duration::from_secs(10)),
            syncstatus_poller: Repeater::every(Duration::from_secs(1)),
            flamegraph_resetter: Repeater::every(Duration::from_secs(60)),
            rollup_config,
            sync_status: Default::default(),
        }
    }
}

impl Data {
    const NUM_DATAPOINTS: usize = 256;
    const SAMPLES_PER_MEDIAN: usize = 128;

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
        match &update {
            Tx::Included(included_in_frag) => {
                if let Some(frag) = self.frags.get_mut(&included_in_frag.frag) {
                    frag.txs.push(uuid)
                } else {
                    tracing::warn!("weird, got an included message for a block we don't know")
                }
            }
            _ => {}
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

    pub fn update(&mut self, consumers: &mut OverseerConsumers, block_time: bool) {
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
                    self.system.push((t, system));
                }
                Telemetry::System(system @ SystemNotification::BlockSync(block_number, gas_used)) => {
                    if !self.blocks.contains_key(&block_number) {
                        let mut block = BlockData::new(block_number, false, Nanos::now());
                        block.gas_used = gas_used;
                        self.blocks.insert(block);
                    } else {
                        let block = self.blocks.get_mut(&block_number).unwrap();
                        // This happens because we got an fcu with a different block than ours at some point and are now resyncing
                        if !block.sealed {
                            block.reset();
                        }
                    }
                    self.system.push((t, system));
                    self.block_number = block_number;
                }
                Telemetry::System(system @ SystemNotification::NewPayload(block_number)) => {
                    if !self.blocks.contains_key(&block_number) {
                        let block = BlockData::new(block_number, false, Nanos::now());
                        self.blocks.insert(block);
                    }
                    self.block_number = block_number;
                    self.system.push((t, system));
                }
                Telemetry::System(system @ SystemNotification::Sorting(block_number)) => {
                    if !self.blocks.contains_key(&block_number) {
                        let block = BlockData::new(block_number, true, Nanos::now());
                        self.blocks.insert(block);
                    }
                    self.block_number = block_number;
                    self.system.push((t, system));
                }
                Telemetry::System(system) => {
                    self.system.push((t, system));
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
            if let Ok(sync_status) =
                consumers.sync_status().inspect_err(|e| tracing::warn!("couldn't get SyncStatus from portal: {e}"))
            {
                self.sync_status = (Nanos::now(), sync_status)
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

    fn current_state(&self) -> Option<(Nanos, SequencerState)> {
        self.system
            .iter()
            .rev()
            .find_map(|(t, s)| if let SystemNotification::StateChanged(state) = s { Some((*t, *state)) } else { None })
    }
    fn last_state(&self) -> Option<(Nanos, SequencerState)> {
        self.system
            .iter()
            .rev()
            .filter_map(
                |(t, s)| if let SystemNotification::StateChanged(state) = s { Some((*t, *state)) } else { None },
            )
            .nth(1)
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
