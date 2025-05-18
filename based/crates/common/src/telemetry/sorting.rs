use alloy_primitives::B256;
use serde::{Deserialize, Serialize};
use strum::{AsRefStr, IntoStaticStr};

use crate::Nanos;

#[derive(Serialize, Deserialize, Clone)]
pub enum WorkerTelemetry {
    Greedy(GreedyWorkerTelemetry),
    LightSpeed(LightSpeedWorkerTelemetry),
    HighPriority(HighPriorityWorkerTelemetry),
    CexDexHotSwap(CexDexHotSwapWorkerTelemetry),
    Submission,
}

impl WorkerTelemetry {
    pub fn on_sorter_start(&self) -> Nanos {
        match self {
            WorkerTelemetry::Greedy(telem_data) => telem_data.on_sorter_start,
            WorkerTelemetry::LightSpeed(telem_data) => telem_data.on_sorter_start,
            WorkerTelemetry::HighPriority(telem_data) => telem_data.on_sorter_start,
            WorkerTelemetry::CexDexHotSwap(telem_data) => telem_data.on_sorter_start,
            _ => Default::default(),
        }
    }

    pub fn on_sorter_finish(&self) -> Nanos {
        match self {
            WorkerTelemetry::Greedy(telem_data) => telem_data.on_sorter_finish,
            WorkerTelemetry::LightSpeed(telem_data) => telem_data.on_sorter_finish,
            WorkerTelemetry::HighPriority(telem_data) => telem_data.on_sorter_finish,
            WorkerTelemetry::CexDexHotSwap(telem_data) => telem_data.on_sorter_finish,
            _ => Default::default(),
        }
    }
    pub fn build_time(&self) -> Nanos {
        self.on_sorter_finish() - self.on_sorter_start()
    }
}

impl From<&WorkerTelemetry> for StrategyTelemetryLight {
    fn from(value: &WorkerTelemetry) -> Self {
        match value {
            WorkerTelemetry::Greedy(t) => StrategyTelemetryLight::Greedy(t.statistics),
            WorkerTelemetry::LightSpeed(t) => StrategyTelemetryLight::LightSpeed(t.statistics),
            WorkerTelemetry::CexDexHotSwap(_) => StrategyTelemetryLight::CexDex,
            WorkerTelemetry::HighPriority(_) => StrategyTelemetryLight::HighPriority,
            WorkerTelemetry::Submission => StrategyTelemetryLight::Submission,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct GreedyWorkerTelemetry {
    pub on_sorter_start: Nanos,
    pub on_aggregatedbundle_apply: Nanos,
    pub on_subsidyboost_apply: Nanos,
    pub on_indepth_sorting_complete: Nanos,
    pub indepth_iterations_completed: u16,
    pub on_high_prio_orders_apply: Nanos,
    pub on_simulate_eob_bundles: Nanos,
    pub on_eob_bundles_apply: Nanos,
    pub on_low_prio_orders_apply: Nanos,
    pub on_eob_backruns_apply: Nanos,
    pub on_sorter_finish: Nanos,
    pub num_orders_to_play_with: u32,
    pub statistics: GreedyTelemetry,
}

impl GreedyWorkerTelemetry {
    /// Create a new `GreedyWorkerTelemetry` with the given start time.
    /// Defaults `on_aggregatedbundle_apply` and `on_subsidyboost_apply` to the start time.
    pub fn new(on_sorter_start: Nanos, num_orders_to_play_with: u32) -> Self {
        Self {
            on_sorter_start,
            num_orders_to_play_with,
            on_aggregatedbundle_apply: on_sorter_start,
            on_subsidyboost_apply: on_sorter_start,
            ..Default::default()
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq, Eq)]
pub struct LightSpeedWorkerTelemetry {
    pub start_block_uuid: B256,
    pub on_sorter_start: Nanos,
    pub on_sort_orders_complete: Nanos,
    pub iterations_completed: u16,
    pub on_sorter_finish: Nanos,
    pub num_orders_to_play_with: u32,
    pub statistics: LightSpeedTelemetry,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq, Eq)]
pub struct HighPriorityWorkerTelemetry {
    pub on_sorter_start: Nanos,
    pub on_sort_orders_complete: Nanos,
    pub on_high_prio_orders_apply: Nanos,
    pub high_prio_iterations_completed: u16,
    pub on_base_orders_apply: Nanos,
    pub base_iterations_completed: u16,
    pub on_sorter_finish: Nanos,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq, Eq)]
pub struct CexDexHotSwapWorkerTelemetry {
    pub start_block_uuid: B256,
    pub on_sorter_start: Nanos,
    pub on_sorter_finish: Nanos,
}

#[derive(Copy, Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[repr(C, packed)]
pub struct GreedyTelemetry {
    pub num_added_in_max_iters: u16,
    pub num_added_in_high_prio: u16,
    pub num_added_in_low_prio: u16,
    pub num_added_in_leftover: u16,
    pub num_eob_sim_requests: u16,
}

#[derive(Copy, Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[repr(C, packed)]
pub struct LightSpeedTelemetry {
    pub new_ords_added: u16,
    // can add more stuff
}

#[derive(Copy, Clone, Debug, IntoStaticStr, AsRefStr, Serialize, Deserialize, PartialEq, Eq)]
#[repr(u8)]
pub enum StrategyTelemetryLight {
    /// Greedy algorithm
    Greedy(GreedyTelemetry),
    /// Given a base block add any new orders to the bottom.
    LightSpeed(LightSpeedTelemetry),
    /// Ultra fast rebuilding for updates from partner cex/dex searchers.
    CexDex,
    /// Given a base block, add high priority orders to the top of the block.
    HighPriority,
    /// Actually submitted block
    Submission,
}

impl Default for StrategyTelemetryLight {
    fn default() -> Self {
        Self::Greedy(Default::default())
    }
}
