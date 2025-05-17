use revm_primitives::B256;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Serialize,PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum SequencerState {
    Syncing,
    WaitingForNewPayload,
    WaitingForForkChoiceWithAttributes,
    Sorting
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[repr(u8)]
pub enum SystemNotification {
    StateChanged(SequencerState),
    BlockSync(u64, u64),
    NewPayload(u64),
    ForkChoiceUpdate(B256),
    Sorting(u64),
    GetPayload(u64),
    BuildStop(u64),
}
