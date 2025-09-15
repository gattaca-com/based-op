use std::ops::Deref;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    communication::{Producer, Queue, queue::QueueType, queues_dir},
    time::Nanos,
};

pub mod frag;
pub use frag::Frag;
pub mod order;
pub use order::Tx;
pub mod system;

pub fn telemetry_queue() -> Queue<TelemetryUpdate> {
    Queue::create_open_or_delete_shared(queues_dir().join("telemetry"), 2usize.pow(18), QueueType::MPMC)
        .expect("Can't create or open telemetry queue")
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[repr(u8)]
pub enum Telemetry {
    Tx(order::Tx),
    Frag(frag::Frag),
    System(system::SystemNotification),
}

#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
pub struct TelemetryUpdate {
    pub identifier: Uuid,
    pub t: Nanos,
    pub update: Telemetry,
}

impl TelemetryUpdate {
    pub fn send(identifier: Uuid, update: Telemetry, producer: &mut Producer<Self>) {
        let msg = Self { identifier, t: Nanos::now(), update };
        producer.produce(&msg);
    }

    pub fn send_ref(identifier: Uuid, update: Telemetry, producer: &Producer<Self>) {
        let msg = Self { identifier, t: Nanos::now(), update };
        producer.produce_without_first(&msg);
    }

    pub fn send_with_time(identifier: Uuid, t: Nanos, update: Telemetry, producer: &mut Producer<Self>) {
        let msg = Self { identifier, t, update };
        producer.produce(&msg);
    }
}

impl Deref for TelemetryUpdate {
    type Target = Telemetry;

    fn deref(&self) -> &Self::Target {
        &self.update
    }
}

impl From<TelemetryUpdate> for (Uuid, Nanos, Telemetry) {
    fn from(value: TelemetryUpdate) -> Self {
        (value.identifier, value.t, value.update)
    }
}
