use serde::{Deserialize, Serialize};

use crate::statistics::Statisticable;

#[derive(Copy, Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Ords(pub u64);

impl Statisticable for Ords {}

impl From<Ords> for u64 {
    fn from(value: Ords) -> Self {
        value.0
    }
}

impl From<u64> for Ords {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

impl std::fmt::Display for Ords {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut v = self.0;
        if v < 1000 {
            return write!(f, "{v}");
        }
        for prefix in ["K", "G"] {
            if v < 1_000_000 {
                return write!(f, "{:.3}{prefix}", (v as f64) / 1000.0);
            }
            v /= 1000;
        }
        write!(f, "{v}M")
    }
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
pub struct PerMillage(pub u64);

impl Statisticable for PerMillage {}

impl From<PerMillage> for u64 {
    fn from(value: PerMillage) -> Self {
        value.0
    }
}

impl From<u64> for PerMillage {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

impl std::fmt::Display for PerMillage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let v = self.0 as f64 / 10.0;
        write!(f, "{v:.1}%")
    }
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Requests(pub u64);

impl Statisticable for Requests {}

impl From<Requests> for u64 {
    fn from(value: Requests) -> Self {
        value.0
    }
}

impl From<u64> for Requests {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

impl std::fmt::Display for Requests {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut v = self.0;
        if v < 1000 {
            return write!(f, "{v}");
        }
        for prefix in ["K", "G"] {
            if v < 1_000_000 {
                return write!(f, "{:.3}{prefix}", (v as f64) / 1000.0);
            }
            v /= 1000;
        }
        write!(f, "{v}M")
    }
}
