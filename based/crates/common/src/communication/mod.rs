use std::{fs::read_dir, path::Path};

use shared_memory::ShmemError;
use thiserror::Error;

pub mod queue;
pub mod seqlock;
pub use queue::{Consumer, Producer, Queue};
pub use seqlock::Seqlock;
pub mod messages;

pub use messages::InternalMessage;

use crate::{
    actor::Actor,
    time::{IngestionTime, Timer},
    utils::last_part_of_typename,
};

// TODO: turn this into a macro
pub trait TrackedSenders {
    fn set_ingestion_t(&mut self, ingestion_t: IngestionTime);
    fn ingestion_t(&self) -> IngestionTime;

    fn send<T>(&self, data: T) -> Result<(), crossbeam_channel::SendError<InternalMessage<T>>>
    where
        Self: AsRef<Sender<T>>,
    {
        let msg = self.ingestion_t().to_msg(data);
        self.as_ref().send(msg)
    }
}

#[derive(Clone, Debug)]
pub struct Receiver<T> {
    receiver: crossbeam_channel::Receiver<InternalMessage<T>>,
    timer:    Timer,
}

impl<T> Receiver<T> {
    pub fn new<S: AsRef<str>>(system_name: S, receiver: crossbeam_channel::Receiver<InternalMessage<T>>) -> Self {
        Self { receiver, timer: Timer::new(format!("{}-{}", system_name.as_ref(), last_part_of_typename::<T>())) }
    }

    #[inline]
    pub fn receive<F, P: TrackedSenders>(&mut self, senders: &mut P, mut f: F) -> bool
    where
        F: FnMut(T, &P),
    {
        if let Ok(m) = self.receiver.try_recv() {
            let ingestion_t: IngestionTime = (&m).into();
            let origin = *ingestion_t.internal();
            senders.set_ingestion_t(ingestion_t);
            self.timer.start();
            f(m.into_data(), senders);
            self.timer.stop_and_latency(origin);
            true
        } else {
            false
        }
    }

    #[inline]
    pub fn receive_raw<F, P: TrackedSenders>(&mut self, senders: &mut P, mut f: F) -> bool
    where
        F: FnMut(InternalMessage<T>, &P),
    {
        if let Ok(m) = self.receiver.try_recv() {
            let ingestion_t: IngestionTime = (&m).into();
            let origin = *ingestion_t.internal();
            senders.set_ingestion_t(ingestion_t);
            self.timer.start();
            f(m, senders);
            self.timer.stop_and_latency(origin);
            true
        } else {
            false
        }
    }
}

pub type Sender<T> = crossbeam_channel::Sender<InternalMessage<T>>;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SequencerToSimulator {
    Ping,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SimulatorToSequencer {
    Pong(usize),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SequencerToRpc {}

#[derive(Debug)]
pub struct ReceiversSequencer {
    from_simulator: Receiver<SimulatorToSequencer>,
    from_rpc:       Receiver<messages::EngineApiMessage>,
}
impl ReceiversSequencer {
    pub fn new<A: Actor>(actor: &A, spine: &Spine) -> Self {
        Self {
            from_simulator: Receiver::new(actor.name(), spine.receiver_sim_to_sequencer.clone()),
            from_rpc:       Receiver::new(actor.name(), spine.receiver_rpc_to_sequencer.clone()),
        }
    }
}

impl AsMut<Receiver<messages::EngineApiMessage>> for ReceiversSequencer {
    fn as_mut(&mut self) -> &mut Receiver<messages::EngineApiMessage> {
        &mut self.from_rpc
    }
}

impl AsMut<Receiver<SimulatorToSequencer>> for ReceiversSequencer {
    fn as_mut(&mut self) -> &mut Receiver<SimulatorToSequencer> {
        &mut self.from_simulator
    }
}

#[derive(Clone, Debug)]
pub struct SendersSequencer {
    to_simulator: Sender<SequencerToSimulator>,
    to_rpc:       Sender<SequencerToRpc>,
    timestamp:    IngestionTime,
}

impl From<&Spine> for SendersSequencer {
    fn from(spine: &Spine) -> Self {
        Self {
            to_simulator: spine.sender_sequencer_to_sim.clone(),
            to_rpc:       spine.sender_sequencer_to_rpc.clone(),
            timestamp:    Default::default(),
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
    timestamp:    IngestionTime,
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

pub struct Connections<S, R> {
    senders:   S,
    receivers: R,
}
impl<S, R> Connections<S, R> {
    pub fn new(senders: S, receivers: R) -> Self {
        Self { senders, receivers }
    }
}

impl<S: TrackedSenders, R> Connections<S, R> {
    #[inline]
    pub fn receive<T, F>(&mut self, mut f: F) -> bool
    where
        R: AsMut<Receiver<T>>,
        F: FnMut(T, &S),
    {
        let receiver = self.receivers.as_mut();
        receiver.receive(&mut self.senders, &mut f)
    }

    #[inline]
    pub fn receive_timestamp<T, F>(&mut self, mut f: F) -> bool
    where
        R: AsMut<Receiver<T>>,
        F: FnMut(InternalMessage<T>, &S),
    {
        let receiver = self.receivers.as_mut();
        receiver.receive_raw(&mut self.senders, &mut f)
    }

    #[inline]
    pub fn send<T>(&mut self, data: T) -> Result<(), crossbeam_channel::SendError<InternalMessage<T>>>
    where
        S: AsRef<Sender<T>>,
    {
        self.senders.set_ingestion_t(IngestionTime::now());
        self.senders.send(data)
    }
}

pub type ConnectionsSequencer = Connections<SendersSequencer, ReceiversSequencer>;
pub type ConnectionsSimulator = Connections<SendersSimulator, ReceiversSimulator>;

// TODO remove
#[allow(dead_code)]
pub struct Spine {
    sender_sim_to_sequencer:   Sender<SimulatorToSequencer>,
    receiver_sim_to_sequencer: crossbeam_channel::Receiver<InternalMessage<SimulatorToSequencer>>,

    sender_sequencer_to_sim:   Sender<SequencerToSimulator>,
    receiver_sequencer_to_sim: crossbeam_channel::Receiver<InternalMessage<SequencerToSimulator>>,

    sender_sequencer_to_rpc:   Sender<SequencerToRpc>,
    receiver_sequencer_to_rpc: crossbeam_channel::Receiver<InternalMessage<SequencerToRpc>>,

    sender_rpc_to_sequencer:   Sender<messages::EngineApiMessage>,
    receiver_rpc_to_sequencer: crossbeam_channel::Receiver<InternalMessage<messages::EngineApiMessage>>,
}

impl Default for Spine {
    fn default() -> Self {
        let (sender_sim_to_sequencer, receiver_sim_to_sequencer) = crossbeam_channel::bounded(4096);
        let (sender_sequencer_to_sim, receiver_sequencer_to_sim) = crossbeam_channel::bounded(4096);
        let (sender_sequencer_to_rpc, receiver_sequencer_to_rpc) = crossbeam_channel::bounded(4096);
        let (sender_rpc_to_sequencer, receiver_rpc_to_sequencer) = crossbeam_channel::bounded(4096);

        Self {
            sender_sim_to_sequencer,
            receiver_sim_to_sequencer,
            sender_sequencer_to_sim,
            receiver_sequencer_to_sim,
            sender_sequencer_to_rpc,
            receiver_sequencer_to_rpc,
            sender_rpc_to_sequencer,
            receiver_rpc_to_sequencer,
        }
    }
}

#[derive(Error, Debug, Copy, Clone, PartialEq)]
pub enum ReadError {
    #[error("Got sped past")]
    SpedPast,
    #[error("Lock empty")]
    Empty,
}

#[derive(Error, Debug)]
#[repr(u8)]
pub enum Error {
    #[error("Queue not initialized")]
    UnInitialized,
    #[error("Queue length not power of two")]
    LengthNotPowerOfTwo,
    #[error("Element size not power of two - 4")]
    ElementSizeNotPowerTwo,
    #[error("Shared memory file does not exist")]
    NonExistingFile,
    #[error("Preexisting shared memory too small")]
    TooSmall,
    #[error("Shmem error")]
    ShmemError(#[from] ShmemError),
}

pub fn clear_shmem<P: AsRef<Path>>(path: P) {
    let path = path.as_ref();
    if !path.exists() {
        return;
    }
    let Ok(mut shmem) = shared_memory::ShmemConf::new().flink(path).open() else {
        return;
    };
    shmem.set_owner(true);
    std::fs::remove_file(path).expect("couldn't remove file");
}

pub fn queues_dir_string() -> String {
    let queues_dir = directories::BaseDirs::new().expect("Couldn't retrieve home dir").data_dir().join("bop/queues");
    queues_dir.to_string_lossy().to_string()
}

pub fn verify_or_remove_queue_files() {
    let queues_dir = directories::BaseDirs::new().expect("Couldn't retrieve home dir").data_dir().join("bop/queues");
    if queues_dir.is_file() {
        let _ = std::fs::remove_file(&queues_dir);
        let _ = std::fs::create_dir_all(queues_dir.as_path());
        return;
    }
    let Ok(files) = read_dir(&queues_dir) else {
        let _ = std::fs::create_dir_all(queues_dir.as_path());
        return;
    };
    for f in files.filter_map(|t| t.ok()) {
        if shared_memory::ShmemConf::new().flink(f.path()).open().is_err() {
            tracing::warn!("couldn't open shmem at {:?} so removing it to be recreated later", f.path());
            let _ = std::fs::remove_file(f.path());
        }
    }
}
