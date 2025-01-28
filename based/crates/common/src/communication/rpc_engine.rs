use super::{
    messages::{self, SequencerToRpc},
    Receiver, Sender, Spine, TrackedSenders,
};
use crate::time::IngestionTime;

#[derive(Debug)]
pub struct ReceiversRpc {
    from_sequencer: Receiver<SequencerToRpc>,
}
impl ReceiversRpc {
    pub fn new<S: AsRef<str>>(actor: S, spine: &Spine) -> Self {
        Self { from_sequencer: Receiver::new(actor, spine.receiver_sequencer_to_rpc.clone()) }
    }
}

impl AsMut<Receiver<SequencerToRpc>> for ReceiversRpc {
    fn as_mut(&mut self) -> &mut Receiver<SequencerToRpc> {
        &mut self.from_sequencer
    }
}

#[derive(Clone, Debug)]
pub struct SendersRpc {
    to_sequencer: Sender<messages::EngineApiMessage>,
    timestamp: IngestionTime,
}

impl From<&Spine> for SendersRpc {
    fn from(spine: &Spine) -> Self {
        Self { to_sequencer: spine.sender_rpc_to_sequencer.clone(), timestamp: Default::default() }
    }
}

impl TrackedSenders for SendersRpc {
    fn set_ingestion_t(&mut self, ingestion_t: IngestionTime) {
        self.timestamp = ingestion_t;
    }

    fn ingestion_t(&self) -> IngestionTime {
        self.timestamp
    }
}
impl AsRef<Sender<messages::EngineApiMessage>> for SendersRpc {
    fn as_ref(&self) -> &Sender<messages::EngineApiMessage> {
        &self.to_sequencer
    }
}
