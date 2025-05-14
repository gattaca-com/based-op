use crate::{
    collections::KeyedCircularBuffer,
    statistics::Statistics,
    ui::{CyclingListState, plot::RenderFlags},
};
use block::BlockData;
use bop_common::{
    communication::{Consumer, Queue, queues_dir_string},
    telemetry::{Telemetry, frag::Frag, order::Tx, system::SystemNotification},
    time::Duration,
    time::TimingMessage,
};
use frag::FragData;
use transaction::TransactionData;

use crate::{
    OverseerConsumers, SLOT_DURATION,
    prelude::*,
    timekeeper::{TimeKeeper, TimerDataState, clock_overhead},
};

pub mod block;
pub mod frag;
pub mod transaction;

#[derive(Clone, Debug, AsRefStr)]
pub enum BuiltBlocksMode {
    BlocksTable(CyclingTableState<B256>),
    DetailsTable(B256, CyclingTableState<B256>, Box<Self>),
    Order(B256, CyclingTableState<B256>, Box<Self>),
}

impl Default for BuiltBlocksMode {
    fn default() -> Self {
        Self::BlocksTable(Default::default())
    }
}

#[derive(Clone, Debug, Default)]
pub struct BuiltBlocksTab {
    pub mode: BuiltBlocksMode,
}

impl BuiltBlocksTab {
    pub(crate) fn select_previous(&mut self) {
        match &mut self.mode {
            BuiltBlocksMode::BlocksTable(state)
            | BuiltBlocksMode::DetailsTable(_, state, _)
            | BuiltBlocksMode::Order(_, state, _) => state.select_previous(),
        }
    }

    pub(crate) fn select_next(&mut self) {
        match &mut self.mode {
            BuiltBlocksMode::BlocksTable(state)
            | BuiltBlocksMode::DetailsTable(_, state, _)
            | BuiltBlocksMode::Order(_, state, _) => state.select_next(),
        }
    }

    pub fn on_enter(&mut self, data: &SlotData) {
        match &self.mode {
            BuiltBlocksMode::BlocksTable(state) => {
                let Some(uuid) = state.selected() else {
                    return;
                };
                self.mode = BuiltBlocksMode::DetailsTable(uuid, Default::default(), Box::new(self.mode.clone()));
            }
            BuiltBlocksMode::DetailsTable(uuid, state, _) => {
                let Some(selected_order) = state.selected() else {
                    return;
                };
                let order_state = CyclingTableState::default().with_prev_slected(*uuid);
                self.mode = BuiltBlocksMode::Order(selected_order, order_state, Box::new(self.mode.clone()));
            }
            BuiltBlocksMode::Order(order_hash, state, _) => {
                let Some(block_uuid) = state.selected() else {
                    return;
                };
                if block_uuid == B256::ZERO {
                    return;
                }
                let mut block_state = CyclingTableState::default().with_prev_slected(*order_hash);
                let block = data.blocks.get(&block_uuid).expect("selected unknown block");
                block_state.select(block.orders.iter().position(|o| o == order_hash));

                self.mode = BuiltBlocksMode::DetailsTable(block_uuid, block_state, Box::new(self.mode.clone()));
            }
        }
    }

    pub fn on_esc(&mut self) {
        match std::mem::take(&mut self.mode) {
            BuiltBlocksMode::BlocksTable(_) => {}
            BuiltBlocksMode::DetailsTable(_, _, prev_mode) | BuiltBlocksMode::Order(_, _, prev_mode) => {
                self.mode = *prev_mode;
            }
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct UIData {
    slot_on_display: Option<u64>,
    pub last_slot: u64,
    pub new_slot: bool,
    pub strat_toggle_list_state: CyclingListState,
    pub built_blocks_tab: BuiltBlocksTab,
    pub strat_toggles_visible: bool,
}

impl UIData {
    pub fn displayed_slot(&self) -> u64 {
        self.slot_on_display.unwrap_or(self.last_slot)
    }

    pub fn update_last_slot(&mut self, slot: u64) {
        self.last_slot = self.last_slot.max(slot);
    }

    pub fn is_current_slot_displayed(&self) -> bool {
        self.slot_on_display.is_none()
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
    block_number: u64,
    pub transactions: KeyedCircularBuffer<TransactionData>,
    pub blocks: KeyedCircularBuffer<BlockData>,
    pub frags: KeyedCircularBuffer<FragData>,
    pub ui_data: UIData,
    pub data_gatherer: Repeater,
    pub timekeeper: TimeKeeper,
    pub time_datas: TimeDatas,
}

impl Default for Data {
    fn default() -> Self {
        Self {
            block_number: Default::default(),
            transactions: Default::default(),
            frags: Default::default(),
            blocks: Default::default(),
            ui_data: UIData::default(),
            timekeeper: Default::default(),
            time_datas: Default::default(),
            data_gatherer: Repeater::every(Duration::from_secs(6) / 256u64),
        }
    }
}

impl Data {
    const NUM_DATAPOINTS: usize = 256;
    const SAMPLES_PER_MEDIAN: usize = 128;

    pub fn is_empty(&self) -> bool {
        self.data_last.strats.is_empty()
    }

    pub fn insert_block(&mut self, key: Uuid, t: Nanos, block: Frag, building: bool, build_start_t: Nanos) {
        let slot = if let Frag::SorterStart { block, .. } = block { block } else { 0 };
        if slot != 0 {
            if self.slot_number != slot {
                self.new_block_t = Nanos::now();
            }
            self.slot_number = slot;
        }

        if let Some(cur) = self.blocks.get_mut(&key) {
            if let Frag::Submission { .. } = block {
                for o in &cur.orders {
                    if let Some(transaction) = self.transactions.get_mut(o) {
                        transaction.submitted.push((t, key))
                    } else {
                        tracing::warn!("Got submitted order that wasn't found in bundles or transactions ")
                    }
                }
            }
            cur.push(t, block);
        }
        if let Frag::SorterStart { .. } = &block {
            let block = BlockData::new(key, t, block);
            self.blocks.insert(block);
        }
    }

    /// Inserts bundle and Optionally returns slot of update
    fn insert_transaction(&mut self, hash: B256, t: Nanos, update: Tx) {
        match &update {
            Tx::Included(included_in_block) => {
                if let Some(block) = self.blocks.get_mut(&included_in_block.block) {
                    block.orders.push(hash)
                } else {
                    tracing::warn!("weid, got an included message for a block we don't know")
                }
            }
            Tx::AddedToPool { .. } => self.strats.available_txs += 1,
            Tx::Removed { .. } => self.strats.available_txs = self.strats.available_txs.saturating_sub(1),
            _ => {}
        }

        if let Some(data) = self.transactions.get_mut(&hash) {
            data.push(t, update);
            return;
        }

        self.transactions.insert(TransactionData::new(hash, t, update));
    }

    pub fn update(&mut self, consumers: &mut OverseerConsumers, slot_time: bool) {
        self.timers.total.start();
        self.timers.orders.start();
        let mut got_one = false;

        while consumers.telemetry.consume(|&mut update| {
            let (key, t, update) = update.into();
            got_one = true;
            match update {
                Telemetry::Tx(tx_update) => {
                    self.data_last.insert_transaction(key, t, tx_update);
                }
                Telemetry::Frag(block_update @ bop_common::telemetry::frag::Frag::SorterStart { .. }) => {
                    self.insert_block(key, t, block_update, self.building, self.build_start_t);
                }
                Telemetry::Frag(block_update) => {
                    self.insert_block(key, t, block_update, self.building, self.build_start_t);
                }
            }
        }) {}
        if got_one {
            self.timers.orders.stop();
        }

        if self.data_gatherer.fired() || slot_time {
            self.data_last.strats.register_datapoint(slot_time);
            self.data_last.bundle_statistics.register_datapoint(slot_time);

            for timer_data in self.data_last.time_datas.data.iter_mut() {
                if let Some(view) = self.ui_data.timekeeper.timers.get_mut(&timer_data.name) {
                    timer_data.register_datapoint(
                        slot_time,
                        self.building,
                        view.latency_consumer.tot_published(),
                        view.processing_consumer.tot_published(),
                    );
                    timer_data.handle_messages(&mut view.latency_consumer, &mut view.processing_consumer);
                }
            }
            self.data_last.set_block_linregs();
        }

        while consumers.system_notifications.consume(|&mut notification| match notification {
            SystemNotification::BuildStop(curslot) => {
                self.building = false;
                self.data_last.slot_number = curslot;
                self.data_last.persist();
                self.update_last_slot(curslot + 1);
            }
            SystemNotification::BuildStart(new_slot) => {
                self.new_slot = true;
                self.building = true;
                self.build_start_t = Nanos::now();
                self.update_last_slot(new_slot);
            }
            _ => {}
        }) {}

        if self.new_slot {
            self.maybe_cleanup_persistent_data();

            for td in self.data_last.time_datas.data.iter_mut().filter(|td| !td.is_empty()) {
                td.reset()
            }
            self.check_new_queues();
        }

        self.timers.total.stop();
    }

    pub fn displayed(&self) -> &SlotData {
        if self.displayed_slot() == self.last_slot { &self.data_last } else { &self.data_selected }
    }

    pub fn on_enter_built_blocks(&mut self) {
        let d = if self.displayed_slot() == self.last_slot { &self.data_last } else { &self.data_selected };
        self.ui_data.built_blocks_tab.on_enter(d);
    }

    pub fn on_enter_cex_dex(&mut self) {
        let d = if self.displayed_slot() == self.last_slot { &self.data_last } else { &self.data_selected };
        self.ui_data.cex_dex_tab.on_enter(d);
    }

    pub fn displayed_mut(&mut self) -> &mut SlotData {
        if self.displayed_slot() == self.last_slot { &mut self.data_last } else { &mut self.data_selected }
    }

    pub fn blocks_visible(&self) -> impl Iterator<Item = &BlockData> {
        self.displayed().blocks.iter().filter(|b| self.is_strat_visible(b.strategy_id()) && b.updates.len() > 1)
    }

    fn clear_stale_data(&mut self) {
        self.bundles.retain(|b| self.slot_number.saturating_sub(b.oldest_slot()) < 100, |_| {});
        self.transactions.retain(|b| self.slot_number.saturating_sub(b.oldest_slot()) < 100, |_| {});
        self.blocks.retain(|b| self.slot_number.saturating_sub(b.slot()) < 100, |_| {});
    }

    pub fn toggle_strat_visibility(&mut self, exclusive: bool) {
        let Some(selected_strat) = self.strat_toggle_list_state.selected() else {
            return;
        };
        if exclusive {
            let is_only = self.strategies[selected_strat].visible
                && !self.strategies.iter().enumerate().any(|(i, s)| i != selected_strat && s.visible);
            for (i, s) in self.strategies.iter_mut().enumerate() {
                s.visible = i == selected_strat || is_only;
            }
        } else {
            self.strategies[selected_strat].visible = !self.strategies[selected_strat].visible;
        }
    }

    pub fn display_next_slot(&mut self) {
        if let Some(displayed) = self.slot_on_display {
            let slot = displayed + 1;
            if slot == self.last_slot { self.slot_on_display = None } else { self.slot_on_display = Some(slot) }
        }
        self.built_blocks_tab.mode = Default::default();
        self.load()
    }

    pub(crate) fn display_prev_slot(&mut self) {
        let displayed = self.slot_on_display.unwrap_or(self.last_slot);
        if displayed == 0 {
            return;
        }
        self.slot_on_display = Some(displayed - 1);
        self.built_blocks_tab.mode = Default::default();
        self.load();
    }

    pub(crate) fn display_slot(&mut self, parsed: u64) {
        self.slot_on_display = Some(parsed);
        self.built_blocks_tab.mode = Default::default();
        self.load();
    }

    pub fn display_last_slot(&mut self) {
        self.ui_data.slot_on_display = None;
        self.built_blocks_tab.mode = Default::default();
        self.load();
    }

    fn check_new_queues(&mut self) {
        let queues_dir: &str = &queues_dir_string();
        let Ok(entries) = std::fs::read_dir(queues_dir) else {
            return;
        };
        for entry in entries.filter_map(|e| e.ok()) {
            let name = entry.path().as_os_str().to_str().unwrap().to_string();
            if name.contains("latency") {
                let (_dir, real_name) = name.split_once("latency-").unwrap();

                if !self.time_datas.contains(real_name) {
                    let latency_q = Queue::open_shared(format!("{queues_dir}/latency-{real_name}"))
                        .expect("couldn't open latency queue");
                    let processing_q = Queue::open_shared(format!("{queues_dir}/timing-{real_name}"))
                        .expect("couldn't open timing queue");
                    let view = TimerDataState::new(latency_q.into(), processing_q.into());
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

impl Deref for Data {
    type Target = UIData;

    fn deref(&self) -> &Self::Target {
        &self.ui_data
    }
}
impl DerefMut for Data {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.ui_data
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

    pub fn clear(&mut self) {
        self.data.clear();
    }

    pub fn toggle_render_options(&mut self, options: RenderFlags) {
        self.data.iter_mut().for_each(|d| d.toggle_render_options(options))
    }
}

