use super::{
    messages::{SequencerToSimulator, SimulatorToSequencer},
    Receiver, Sender, Spine, TrackedSenders,
};
use crate::{actor::Actor, time::IngestionTime};

#[derive(Debug)]
pub struct ReceiversSimulator<Db> {
    from_sequencer: Receiver<SequencerToSimulator<Db>>,
}
impl<Db> ReceiversSimulator<Db> {
    pub fn new<A: Actor>(actor: &A, spine: &Spine<Db>) -> Self {
        Self { from_sequencer: Receiver::new(actor.name(), spine.receiver_sequencer_to_sim.clone()) }
    }
}

impl<Db> AsMut<Receiver<SequencerToSimulator<Db>>> for ReceiversSimulator<Db> {
    fn as_mut(&mut self) -> &mut Receiver<SequencerToSimulator<Db>> {
        &mut self.from_sequencer
    }
}

#[derive(Clone, Debug)]
pub struct SendersSimulator {
    to_sequencer: Sender<SimulatorToSequencer>,
    timestamp: IngestionTime,
}

impl<Db> From<&Spine<Db>> for SendersSimulator {
    fn from(spine: &Spine<Db>) -> Self {
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
