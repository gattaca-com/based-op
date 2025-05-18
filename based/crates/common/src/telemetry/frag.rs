use serde::{Deserialize, Serialize};
use strum_macros::AsRefStr;

use crate::eth::MicroEth;

#[derive(Copy, Clone, Debug, Serialize, Deserialize, AsRefStr, PartialEq, Eq)]
#[repr(u8)]
pub enum Frag {
    SorterStart { block: u64, seq: u64, available_value: MicroEth },
    SorterFinish { success: bool, payment: MicroEth, best_order_value: MicroEth, n_txs: usize, gas_used: u64 },
    Commit,
}
