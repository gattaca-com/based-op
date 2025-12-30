use alloy_primitives::Address;
use revm_primitives::B256;
use serde::{Deserialize, Serialize};
use strum_macros::AsRefStr;
use uuid::Uuid;

use crate::{eth::MicroEth, time::Nanos};

#[derive(Clone, Copy, Debug, PartialEq, PartialOrd, Ord, Eq, Default, Serialize, Deserialize)]
pub struct TransactionInclusion {
    pub frag: Uuid,
    pub id_in_frag: u16,
    pub payment: MicroEth,
    pub sim_time: Nanos,
    pub gas_used: u64,
    pub bundle_id: Option<Uuid>,
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
    Included(TransactionInclusion),
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

#[derive(Clone, Debug, PartialEq, PartialOrd, Ord, Eq, Default, Serialize, Deserialize)]
pub struct BundleInclusion {
    pub frag: Uuid,
    pub txs: Vec<(Uuid, TransactionInclusion)>,
}

impl BundleInclusion {
    pub fn payment(&self) -> MicroEth {
        let mut payment = MicroEth::default();
        for tx in self.txs.iter() {
            payment += tx.1.payment;
        }

        payment
    }

    pub fn gas_used(&self) -> u64 {
        self.txs.iter().map(|tx| tx.1.gas_used).sum()
    }

    pub fn sim_time(&self) -> Nanos {
        self.txs.iter().map(|tx| tx.1.sim_time).sum()
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, AsRefStr)]
#[repr(u8)]
pub enum Bundle {
    Ingested { uuid: Uuid, signer: Option<Address> },
    AddedToPool,
    Included(BundleInclusion),
    RemovedFromPool,
}
