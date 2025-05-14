use alloy_primitives::Address;
use revm_primitives::B256;
use serde::{Deserialize, Serialize};
use strum_macros::AsRefStr;
use uuid::Uuid;

use crate::{eth::MicroEth, time::Nanos};

#[derive(Clone, Copy, Debug, PartialEq, PartialOrd, Ord, Eq, Default, Serialize, Deserialize)]
pub struct IncludedInFrag {
    pub frag: Uuid,
    pub id_in_frag: u16,
    pub payment: MicroEth,
    pub sim_time: Nanos,
    pub gas_used: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, PartialOrd, Ord, Eq, Default, Serialize, Deserialize)]
pub struct Ingested {
    pub sender: Address,
    pub nonce: u64,
    pub hash: B256,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, AsRefStr)]
#[repr(u8)]
pub enum Tx {
    Ingested(Ingested),
    AddedToPool,
    Included(IncludedInFrag),
    RemovedFromPool,
}
impl Tx {
    pub fn id_in_frag(&self) -> Option<usize> {
        match self {
            Tx::Included(included_in_frag) => Some(included_in_frag.id_in_frag as usize),
            _ => None,
        }
    }
}
