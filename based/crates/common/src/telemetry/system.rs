use revm_primitives::B256;
use serde::{Deserialize, Serialize};
use strum_macros::AsRefStr;

#[derive(Default, Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, AsRefStr)]
#[repr(u8)]
pub enum SequencerState {
    #[default]
    Unknown,
    Syncing,
    WaitingForNewPayload,
    WaitingForForkChoiceWithAttributes,
    Sorting,
}

impl ToString for SequencerState {
    fn to_string(&self) -> String {
        format!("{}", self.as_ref())
    }
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
impl Default for SystemNotification {
    fn default() -> Self {
        Self::StateChanged(Default::default())
    }
}
