use super::{messages::EthApi, Sender, Spine, TrackedSenders};
use crate::time::IngestionTime;

#[derive(Clone, Debug)]
pub struct SendersEthRpc {
    to_sequencer: Sender<EthApi>,
    timestamp: IngestionTime,
}

impl From<&Spine> for SendersEthRpc {
    fn from(spine: &Spine) -> Self {
        Self { to_sequencer: spine.sender_eth_rpc_to_sequencer.clone(), timestamp: Default::default() }
    }
}

impl TrackedSenders for SendersEthRpc {
    fn set_ingestion_t(&mut self, ingestion_t: IngestionTime) {
        self.timestamp = ingestion_t;
    }

    fn ingestion_t(&self) -> IngestionTime {
        self.timestamp
    }
}
impl AsRef<Sender<EthApi>> for SendersEthRpc {
    fn as_ref(&self) -> &Sender<EthApi> {
        &self.to_sequencer
    }
}
