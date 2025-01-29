use std::sync::Arc;

use super::{
    messages::{self, SequencerToRpc, SequencerToSimulator, SimulatorToSequencer},
    Receiver, Sender, Spine, TrackedSenders,
};
use crate::{actor::Actor, time::IngestionTime, transaction::Transaction};

#[derive(Debug)]
pub struct ReceiversSequencer {
    from_simulator: Receiver<SimulatorToSequencer>,
    from_engine_rpc: Receiver<messages::EngineApi>,
    from_eth_rpc: Receiver<Arc<Transaction>>,
}
impl ReceiversSequencer {
    pub fn new<A: Actor, Db>(actor: &A, spine: &Spine<Db>) -> Self {
        Self {
            from_simulator: Receiver::new(actor.name(), spine.receiver_sim_to_sequencer.clone()),
            from_engine_rpc: Receiver::new(actor.name(), spine.receiver_engine_rpc_to_sequencer.clone()),
            from_eth_rpc: Receiver::new(actor.name(), spine.receiver_eth_rpc_to_sequencer.clone()),
        }
    }
}

impl AsMut<Receiver<messages::EngineApi>> for ReceiversSequencer {
    fn as_mut(&mut self) -> &mut Receiver<messages::EngineApi> {
        &mut self.from_engine_rpc
    }
}

impl AsMut<Receiver<SimulatorToSequencer>> for ReceiversSequencer {
    fn as_mut(&mut self) -> &mut Receiver<SimulatorToSequencer> {
        &mut self.from_simulator
    }
}

impl AsMut<Receiver<Arc<Transaction>>> for ReceiversSequencer {
    fn as_mut(&mut self) -> &mut Receiver<Arc<Transaction>> {
        &mut self.from_eth_rpc
    }
}

#[derive(Clone, Debug)]
pub struct SendersSequencer<Db> {
    to_simulator: Sender<SequencerToSimulator<Db>>,
    to_rpc: Sender<SequencerToRpc>,
    timestamp: IngestionTime,
}

impl<Db> From<&Spine<Db>> for SendersSequencer<Db> {
    fn from(spine: &Spine<Db>) -> Self {
        Self {
            to_simulator: spine.sender_sequencer_to_sim.clone(),
            to_rpc: spine.sender_sequencer_to_rpc.clone(),
            timestamp: Default::default(),
        }
    }
}

impl<Db> AsRef<Sender<SequencerToSimulator<Db>>> for SendersSequencer<Db> {
    fn as_ref(&self) -> &Sender<SequencerToSimulator<Db>> {
        &self.to_simulator
    }
}

impl<Db> AsRef<Sender<SequencerToRpc>> for SendersSequencer<Db> {
    fn as_ref(&self) -> &Sender<SequencerToRpc> {
        &self.to_rpc
    }
}

impl<Db> TrackedSenders for SendersSequencer<Db> {
    fn set_ingestion_t(&mut self, ingestion_t: IngestionTime) {
        self.timestamp = ingestion_t;
    }

    fn ingestion_t(&self) -> IngestionTime {
        self.timestamp
    }
}
