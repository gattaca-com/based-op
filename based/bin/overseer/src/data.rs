use crate::{collections::KeyedCircularBuffer, statistics::Statistics, ui::plot::RenderFlags};
use block::BlockData;
use bop_common::{
    communication::{Consumer, Queue, queues_dir},
    telemetry::{Telemetry, frag::Frag, order::Tx, system::SystemNotification},
    time::Duration,
    time::TimingMessage,
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
}

impl UIData {
    pub fn render_overview(&mut self, data: &Data, frame: &mut Frame) {
        let [left, middle, right] =
            Layout::horizontal([Constraint::Percentage(33), Constraint::Percentage(33), Constraint::Fill(1)])
                .areas(frame.area());
        self.table_blocks.render(
            Some("Blocks".to_string()),
            BlockData::header(),
            data.blocks.iter().map(|b| b.to_row()).rev(),
            frame,
            left,
        );
        self.table_frags.render(
            Some("Frags in block".to_string()),
            FragData::block_table_header(),
            data.frags.iter().map(|b| b.to_block_table_row()).rev(),
            frame,
            middle,
        );
        self.table_pool.render(
            Some("Tx Pool".to_string()),
            TransactionData::pool_header(),
            data.transactions.iter().map(|b| b.to_pool_row()).rev(),
            frame,
            right,
        )
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
    pub data_gatherer: Repeater,
    pub queue_checker: Repeater,
    pub flamegraph_resetter: Repeater,
    pub timekeeper: TimeKeeper,
    pub time_datas: TimeDatas,
}

impl Default for Data {
    fn default() -> Self {
        Self {
            block_number: Default::default(),
            transactions: KeyedCircularBuffer::new(30_000),
            frags: KeyedCircularBuffer::new(30_000),
            blocks: KeyedCircularBuffer::new(30_000),
            timekeeper: Default::default(),
            time_datas: Default::default(),
            data_gatherer: Repeater::every(Duration::from_secs(6) / 256u64),
            queue_checker: Repeater::every(Duration::from_secs(10)),
            flamegraph_resetter: Repeater::every(Duration::from_secs(60)),
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
                    tracing::warn!("weid, got an included message for a block we don't know")
                }
            }
            _ => {}
        }

        if let Some(data) = self.transactions.get_mut(&uuid) {
            data.push(t, update);
            return;
        }

        self.transactions.insert(TransactionData::new(uuid, t, update));
    }

    pub fn update(&mut self, consumers: &mut OverseerConsumers, block_time: bool) {
        while let Some(update) = consumers.telemetry.try_consume() {
            tracing::info!("got a message");
            let (key, t, update) = update.into();
            match update {
                Telemetry::Tx(tx_update) => {
                    self.insert_transaction(key, t, tx_update);
                }
                Telemetry::Frag(update) => {
                    self.insert_frag(t, key, update);
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

        while let Some(notification) = consumers.system_notifications.try_consume() {
            match notification {
                SystemNotification::BuildStop(curblock) => {
                    self.block_number = curblock;
                }
                SystemNotification::BlockSync(block_number, gas_used) => {
                    if !self.blocks.contains_key(&block_number) {
                        let mut block = BlockData::new(block_number, false, Nanos::now());
                        block.gas_used = gas_used;
                        self.blocks.insert(block);
                    }
                    self.block_number = block_number;
                }
                _ => {}
            }
        }
        if self.queue_checker.fired() {
            self.check_new_queues();
        }
        if self.flamegraph_resetter.fired() {
            self.time_datas.reset()
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
