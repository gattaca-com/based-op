use super::{
    messages::{SequencerToSimulator, SimulatorToSequencer},
    Receiver, Sender, Spine, TrackedSenders,
};
use crate::{actor::Actor, time::IngestionTime};

#[derive(Debug)]
pub struct ReceiversSimulator {
    from_sequencer: Receiver<SequencerToSimulator>,
}
impl ReceiversSimulator {
    pub fn new<A: Actor>(actor: &A, spine: &Spine) -> Self {
        Self { from_sequencer: Receiver::new(actor.name(), spine.receiver_sequencer_to_sim.clone()) }
    }
}

impl AsMut<Receiver<SequencerToSimulator>> for ReceiversSimulator {
    fn as_mut(&mut self) -> &mut Receiver<SequencerToSimulator> {
        &mut self.from_sequencer
    }
}

#[derive(Clone, Debug)]
pub struct SendersSimulator {
    to_sequencer: Sender<SimulatorToSequencer>,
    timestamp: IngestionTime,
}

impl From<&Spine> for SendersSimulator {
    fn from(spine: &Spine) -> Self {
        Self { to_sequencer: spine.sender_sim_to_sequencer.clone(), timestamp: Default::default() }
    }
}

impl TrackedSenders for SendersSimulator {
    fn set_ingestion_t(&mut self, ingestion_t: IngestionTime) {
        self.timestamp = ingestion_t;
    }

    fn ingestion_t(&self) -> IngestionTime {
        self.timestamp
    }
}
impl AsRef<Sender<SimulatorToSequencer>> for SendersSimulator {
    fn as_ref(&self) -> &Sender<SimulatorToSequencer> {
        &self.to_sequencer
    }
}
