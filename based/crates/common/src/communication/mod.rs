use std::{fs::read_dir, path::Path};

use messages::{EthApi, SequencerToRpc, SequencerToSimulator, SimulatorToSequencer};
use shared_memory::ShmemError;
use thiserror::Error;

pub mod queue;
pub mod seqlock;
pub use queue::{Consumer, Producer, Queue};
pub use seqlock::Seqlock;
pub mod messages;
pub mod rpc_engine;
pub mod rpc_eth;
pub mod sequencer;
pub mod simulator;

pub use messages::InternalMessage;

use crate::{
    time::{Duration, IngestionTime, Instant, Timer},
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

    fn send_log_err<T>(&self, data: T) -> Result<(), crossbeam_channel::SendError<InternalMessage<T>>>
    where
        Self: AsRef<Sender<T>>,
    {
        self.send(data).inspect_err(|e| tracing::error!("Couldn't send {}: {e}", last_part_of_typename::<T>()))
    }

    fn send_forever<T>(&self, data: T)
    where
        Self: AsRef<Sender<T>>,
    {
        if let Err(e) = self.send(data) {
            tracing::error!("Couldn't send {}: {e}, retrying forever...", last_part_of_typename::<T>());
            let mut msg = e.into_inner();
            while let Err(e) = self.as_ref().send(msg) {
                msg = e.into_inner();
            }
        }
    }

    fn send_timeout<T>(
        &self,
        data: T,
        timeout: Duration,
    ) -> Result<(), crossbeam_channel::SendError<InternalMessage<T>>>
    where
        Self: AsRef<Sender<T>>,
    {
        if let Err(e) = self.send(data) {
            tracing::error!("Couldn't send {}: {e}, retrying for {timeout}...", last_part_of_typename::<T>());
            let curt = Instant::now();
            let mut msg = e.into_inner();
            while let Err(e) = self.as_ref().send(msg) {
                if timeout < curt.elapsed() {
                    tracing::error!(
                        "Couldn't send {}: {e}, retried for {timeout}, breaking off",
                        last_part_of_typename::<T>()
                    );
                    return Err(e);
                }
                msg = e.into_inner();
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct Receiver<T> {
    receiver: crossbeam_channel::Receiver<InternalMessage<T>>,
    timer: Timer,
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

pub struct Connections<S, R> {
    senders: S,
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

// TODO remove
#[allow(dead_code)]
pub struct Spine {
    sender_sim_to_sequencer: Sender<SimulatorToSequencer>,
    receiver_sim_to_sequencer: crossbeam_channel::Receiver<InternalMessage<SimulatorToSequencer>>,

    sender_sequencer_to_sim: Sender<SequencerToSimulator>,
    receiver_sequencer_to_sim: crossbeam_channel::Receiver<InternalMessage<SequencerToSimulator>>,

    sender_sequencer_to_rpc: Sender<SequencerToRpc>,
    receiver_sequencer_to_rpc: crossbeam_channel::Receiver<InternalMessage<SequencerToRpc>>,

    sender_engine_rpc_to_sequencer: Sender<messages::EngineApi>,
    receiver_engine_rpc_to_sequencer: crossbeam_channel::Receiver<InternalMessage<messages::EngineApi>>,

    //TODO: @ltitanb
    sender_eth_rpc_to_sequencer: Sender<EthApi>,
    receiver_eth_rpc_to_sequencer: crossbeam_channel::Receiver<InternalMessage<EthApi>>,
}

impl Default for Spine {
    fn default() -> Self {
        let (sender_sim_to_sequencer, receiver_sim_to_sequencer) = crossbeam_channel::bounded(4096);
        let (sender_sequencer_to_sim, receiver_sequencer_to_sim) = crossbeam_channel::bounded(4096);
        let (sender_sequencer_to_rpc, receiver_sequencer_to_rpc) = crossbeam_channel::bounded(4096);
        let (sender_engine_rpc_to_sequencer, receiver_engine_rpc_to_sequencer) = crossbeam_channel::bounded(4096);
        let (sender_eth_rpc_to_sequencer, receiver_eth_rpc_to_sequencer) = crossbeam_channel::bounded(4096);

        Self {
            sender_sim_to_sequencer,
            receiver_sim_to_sequencer,
            sender_sequencer_to_sim,
            receiver_sequencer_to_sim,
            sender_sequencer_to_rpc,
            receiver_sequencer_to_rpc,
            sender_engine_rpc_to_sequencer,
            receiver_engine_rpc_to_sequencer,
            sender_eth_rpc_to_sequencer,
            receiver_eth_rpc_to_sequencer,
        }
    }
}

impl From<&Spine> for Sender<messages::EngineApi> {
    fn from(value: &Spine) -> Self {
        value.sender_engine_rpc_to_sequencer.clone()
    }
}

impl From<&Spine> for Sender<messages::EthApi> {
    fn from(value: &Spine) -> Self {
        value.sender_eth_rpc_to_sequencer.clone()
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
