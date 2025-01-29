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
    pub fn new<A: Actor>(actor: &A, spine: &Spine) -> Self {
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
pub struct SendersSequencer {
    to_simulator: Sender<SequencerToSimulator>,
    to_rpc: Sender<SequencerToRpc>,
    timestamp: IngestionTime,
}

impl From<&Spine> for SendersSequencer {
    fn from(spine: &Spine) -> Self {
        Self {
            to_simulator: spine.sender_sequencer_to_sim.clone(),
            to_rpc: spine.sender_sequencer_to_rpc.clone(),
            timestamp: Default::default(),
        }
    }
}

impl AsRef<Sender<SequencerToSimulator>> for SendersSequencer {
    fn as_ref(&self) -> &Sender<SequencerToSimulator> {
        &self.to_simulator
    }
}

impl AsRef<Sender<SequencerToRpc>> for SendersSequencer {
    fn as_ref(&self) -> &Sender<SequencerToRpc> {
        &self.to_rpc
    }
}

impl TrackedSenders for SendersSequencer {
    fn set_ingestion_t(&mut self, ingestion_t: IngestionTime) {
        self.timestamp = ingestion_t;
    }

    fn ingestion_t(&self) -> IngestionTime {
        self.timestamp
    }
}
