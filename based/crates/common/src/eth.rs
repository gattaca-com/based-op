use std::ops::{Add, AddAssign, Div};

use revm_primitives::U256;
use serde::{Deserialize, Serialize};

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct MicroEth(pub u32);

pub fn to_micro_eth(wei: U256) -> u32 {
    u32::try_from(wei / U256::from_limbs([1_000_000_000_000, 0, 0, 0]))
        .inspect_err(|_| tracing::warn!("value {:?} was too large to fit in u32 in telemetry message", wei))
        .unwrap_or_default()
}
impl From<U256> for MicroEth {
    fn from(value: U256) -> Self {
        Self(to_micro_eth(value))
    }
}

impl std::fmt::Display for MicroEth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", scientific_notation_from_micro(self.0 as u64))
    }
}

impl Add for MicroEth {
    type Output = MicroEth;

    fn add(self, rhs: Self) -> Self::Output {
        MicroEth(self.0 + rhs.0)
    }
}

impl AddAssign for MicroEth {
    fn add_assign(&mut self, rhs: Self) {
        self.0 += rhs.0;
    }
}

impl Div<usize> for MicroEth {
    type Output = MicroEth;

    fn div(self, rhs: usize) -> Self::Output {
        MicroEth(self.0 / rhs as u32)
    }
}

pub fn scientific_notation_from_micro(mut v: u64) -> String {
    if v < 1000 {
        return format!("{v}μ");
    }
    for prefix in ["m", "", "K", "M", "G", "T", "P"] {
        if v < 1_000_000 {
            return format!("{:.3}{prefix}", v as f64 / 1000.0);
        }
        v /= 1000;
    }
    format!("{v}M")
}


