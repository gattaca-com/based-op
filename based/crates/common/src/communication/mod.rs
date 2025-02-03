use std::{fs::read_dir, marker::PhantomData, path::Path, sync::Arc};

use messages::{BlockSyncMessage, SequencerToExternal, SequencerToSimulator, SimulatorToSequencer};
use revm_primitives::BlockEnv;
use shared_memory::ShmemError;
use thiserror::Error;

pub mod queue;
pub mod seqlock;
pub use queue::{Consumer, Producer, Queue};
pub use seqlock::Seqlock;
pub mod messages;
pub use messages::InternalMessage;

use crate::{
    db::BopDbRead,
    p2p::FragMessage,
    time::{Duration, IngestionTime, Instant, Timer},
    transaction::Transaction,
    utils::last_part_of_typename,
};

pub type CrossBeamReceiver<T> = crossbeam_channel::Receiver<InternalMessage<T>>;

pub trait NonBlockingSender<T> {
    fn try_send(&self, data: T) -> Result<(), T>;
}

pub trait HasSender<T> {
    type Sender: NonBlockingSender<InternalMessage<T>>;
    fn get_sender(&self) -> &Self::Sender;
}

pub trait NonBlockingReceiver<T> {
    fn try_receive(&mut self) -> Option<T>;
}

// TODO: turn this into a macro
pub trait TrackedSenders {
    fn set_ingestion_t(&mut self, ingestion_t: IngestionTime);
    fn ingestion_t(&self) -> IngestionTime;

    fn send<T>(&self, data: T) -> Result<(), InternalMessage<T>>
    where
        Self: HasSender<T>,
    {
        let msg = self.ingestion_t().to_msg(data);
        self.get_sender().try_send(msg)
    }

    fn send_forever<T>(&self, data: T)
    where
        Self: HasSender<T>,
    {
        if let Err(e) = self.send(data) {
            tracing::error!("Couldn't send {}: retrying forever...", last_part_of_typename::<T>());
            let mut msg = e.into_data();
            while let Err(e) = self.send(msg) {
                msg = e.into_data();
            }
        }
    }

    fn send_timeout<T>(&self, data: T, timeout: Duration) -> Result<(), InternalMessage<T>>
    where
        Self: HasSender<T>,
    {
        if let Err(e) = self.send(data) {
            tracing::error!("Couldn't send {}: retrying for {timeout}...", last_part_of_typename::<T>());
            let curt = Instant::now();
            let mut msg = e.into_data();
            while let Err(e) = self.send(msg) {
                if timeout < curt.elapsed() {
                    tracing::error!(
                        "Couldn't send {}: retried for {timeout}, breaking off",
                        last_part_of_typename::<T>()
                    );
                    return Err(e);
                }
                msg = e.into_data();
            }
        }
        Ok(())
    }
}

impl<T> NonBlockingSender<T> for crossbeam_channel::Sender<T> {
    fn try_send(&self, data: T) -> Result<(), T> {
        self.send(data).map_err(|e| e.into_inner())
    }
}

impl<T> NonBlockingReceiver<T> for crossbeam_channel::Receiver<T> {
    fn try_receive(&mut self) -> Option<T> {
        self.try_recv().ok()
    }
}

impl<T: Clone> NonBlockingSender<T> for Producer<T> {
    fn try_send(&self, data: T) -> Result<(), T> {
        self.produce_without_first(&data);
        Ok(())
    }
}

impl<T: Clone> NonBlockingReceiver<T> for Consumer<T> {
    fn try_receive(&mut self) -> Option<T> {
        self.try_consume()
    }
}

#[derive(Clone, Debug)]
pub struct Receiver<T, R = CrossBeamReceiver<T>> {
    receiver: R,
    timer: Timer,
    _t: PhantomData<T>,
}

impl<T, R: NonBlockingReceiver<InternalMessage<T>>> Receiver<T, R> {
    pub fn new<S: AsRef<str>>(system_name: S, receiver: R) -> Self {
        Self {
            receiver,
            timer: Timer::new(format!("{}-{}", system_name.as_ref(), last_part_of_typename::<T>())),
            _t: PhantomData,
        }
    }

    #[inline]
    pub fn receive<F, P: TrackedSenders>(&mut self, senders: &mut P, mut f: F) -> bool
    where
        F: FnMut(T, &P),
    {
        if let Some(m) = self.receiver.try_receive() {
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
        if let Some(m) = self.receiver.try_receive() {
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

    pub fn senders(&self) -> &S {
        &self.senders
    }
}

impl<S: TrackedSenders, R> Connections<S, R> {
    #[inline]
    pub fn receive<T, F, RR>(&mut self, mut f: F) -> bool
    where
        RR: NonBlockingReceiver<InternalMessage<T>>,
        R: AsMut<Receiver<T, RR>>,
        F: FnMut(T, &S),
    {
        let receiver = self.receivers.as_mut();
        receiver.receive(&mut self.senders, &mut f)
    }

    #[inline]
    pub fn receive_timestamp<T, F, RR>(&mut self, mut f: F) -> bool
    where
        RR: NonBlockingReceiver<InternalMessage<T>>,
        R: AsMut<Receiver<T, RR>>,
        F: FnMut(InternalMessage<T>, &S),
    {
        let receiver = self.receivers.as_mut();
        receiver.receive_raw(&mut self.senders, &mut f)
    }

    #[inline]
    pub fn send<T>(&mut self, data: T) -> Result<(), InternalMessage<T>>
    where
        S: HasSender<T>,
    {
        self.senders.set_ingestion_t(IngestionTime::now());
        self.senders.send(data)
    }

    pub fn set_ingestion_t(&mut self, ingestion_t: IngestionTime) {
        self.senders.set_ingestion_t(ingestion_t);
    }
}

#[derive(Clone)]
pub struct Spine<Db: BopDbRead> {
    sender_simulator_to_sequencer: Sender<SimulatorToSequencer<Db>>,
    receiver_simulator_to_sequencer: CrossBeamReceiver<SimulatorToSequencer<Db>>,

    sender_sequencer_to_simulator: Sender<SequencerToSimulator<Db>>,
    receiver_sequencer_to_simulator: CrossBeamReceiver<SequencerToSimulator<Db>>,

    sender_sequencer_to_rpc: Sender<SequencerToExternal>,
    receiver_sequencer_to_rpc: CrossBeamReceiver<SequencerToExternal>,

    sender_engine_rpc_to_sequencer: Sender<messages::EngineApi>,
    receiver_engine_rpc_to_sequencer: CrossBeamReceiver<messages::EngineApi>,

    sender_eth_rpc_to_sequencer: Sender<Arc<Transaction>>,
    receiver_eth_rpc_to_sequencer: CrossBeamReceiver<Arc<Transaction>>,

    sender_blockfetch_to_sequencer: Sender<BlockSyncMessage>,
    receiver_blockfetch_to_sequencer: CrossBeamReceiver<BlockSyncMessage>,

    sender_sequencer_frag_broadcast: Sender<FragMessage>,
    receiver_sequencer_frag_broadcast: CrossBeamReceiver<FragMessage>,

    blockenv: Queue<InternalMessage<BlockEnv>>,
}

impl<Db: BopDbRead> Default for Spine<Db> {
    fn default() -> Self {
        let (sender_simulator_to_sequencer, receiver_simulator_to_sequencer) = crossbeam_channel::bounded(4096);
        let (sender_sequencer_to_simulator, receiver_sequencer_to_simulator) = crossbeam_channel::bounded(4096);
        let (sender_sequencer_to_rpc, receiver_sequencer_to_rpc) = crossbeam_channel::bounded(4096);
        let (sender_engine_rpc_to_sequencer, receiver_engine_rpc_to_sequencer) = crossbeam_channel::bounded(4096);
        let (sender_eth_rpc_to_sequencer, receiver_eth_rpc_to_sequencer) = crossbeam_channel::bounded(4096);
        let (sender_blockfetch_to_sequencer, receiver_blockfetch_to_sequencer) = crossbeam_channel::bounded(4096);
        let (sender_sequencer_frag_broadcast, receiver_sequencer_frag_broadcast) = crossbeam_channel::bounded(4096);

        // MPMC to be safe, should only be produced to by the sequencer but
        let blockenv = Queue::new(4096, queue::QueueType::MPMC).expect("couldn't initialize queue");
        Self {
            sender_simulator_to_sequencer,
            receiver_simulator_to_sequencer,
            sender_sequencer_to_simulator,
            receiver_sequencer_to_simulator,
            sender_sequencer_to_rpc,
            receiver_sequencer_to_rpc,
            sender_engine_rpc_to_sequencer,
            receiver_engine_rpc_to_sequencer,
            sender_eth_rpc_to_sequencer,
            receiver_eth_rpc_to_sequencer,
            sender_blockfetch_to_sequencer,
            receiver_blockfetch_to_sequencer,
            sender_sequencer_frag_broadcast,
            receiver_sequencer_frag_broadcast,
            blockenv,
        }
    }
}

impl<Db: BopDbRead> Spine<Db> {
    pub fn to_connections<S: AsRef<str>>(&self, name: S) -> SpineConnections<Db> {
        SpineConnections::new(self.into(), ReceiversSpine::attach(name, self))
    }
}

macro_rules! from_spine {
    ($T:ty, $v:ident, $S: tt) => {
        paste::item! {
            impl<Db: BopDbRead> From<&Spine<Db>> for Sender<$T> {
                fn from(spine: &Spine<Db>) -> Self {
                    spine.[<sender_ $v>].clone()
                }
            }

            impl<Db: BopDbRead> From<&Spine<Db>> for CrossBeamReceiver<$T> {
                fn from(spine: &Spine<Db>) -> Self {
                    spine.[<receiver_ $v>].clone()
                }
            }

            impl<Db: BopDbRead> AsRef<Sender<$T>> for SendersSpine<Db> {
                fn as_ref(&self) -> &Sender<$T> {
                    &self.$v
                }
            }

            impl<Db: BopDbRead> HasSender<$T> for SendersSpine<Db> {
                type Sender = $S<$T>;
                fn get_sender(&self) -> &Self::Sender {
                    &self.$v
                }
            }

            impl<Db: BopDbRead> From<&'_ SendersSpine<Db>> for Sender<$T> {
                fn from(value: &'_ SendersSpine<Db>) -> Self {
                    value.$v.clone()
                }
            }
            impl<Db: BopDbRead> AsMut<Receiver<$T>> for ReceiversSpine<Db> {
                fn as_mut(&mut self) -> &mut Receiver<$T> {
                    &mut self.$v
                }
            }
        }
    };
}

from_spine!(FragMessage, sequencer_frag_broadcast, Sender);
from_spine!(SimulatorToSequencer<Db>, simulator_to_sequencer, Sender);
from_spine!(SequencerToSimulator<Db>, sequencer_to_simulator, Sender);
from_spine!(SequencerToExternal, sequencer_to_rpc, Sender);
from_spine!(messages::EngineApi, engine_rpc_to_sequencer, Sender);
from_spine!(Arc<Transaction>, eth_rpc_to_sequencer, Sender);
from_spine!(BlockSyncMessage, blockfetch_to_sequencer, Sender);

impl<Db: BopDbRead> HasSender<BlockEnv> for SendersSpine<Db> {
    type Sender = Producer<InternalMessage<BlockEnv>>;

    fn get_sender(&self) -> &Self::Sender {
        &self.blockenv
    }
}

impl<Db: BopDbRead> AsMut<Receiver<BlockEnv, Consumer<InternalMessage<BlockEnv>>>> for ReceiversSpine<Db> {
    fn as_mut(&mut self) -> &mut Receiver<BlockEnv, Consumer<InternalMessage<BlockEnv>>> {
        &mut self.blockenv
    }
}


//TODO: remove allow dead code
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct SendersSpine<Db: BopDbRead> {
    sequencer_to_simulator: Sender<SequencerToSimulator<Db>>,
    sequencer_to_rpc: Sender<SequencerToExternal>,
    simulator_to_sequencer: Sender<SimulatorToSequencer<Db>>,
    engine_rpc_to_sequencer: Sender<messages::EngineApi>,
    eth_rpc_to_sequencer: Sender<Arc<Transaction>>,
    blockfetch_to_sequencer: Sender<BlockSyncMessage>,
    sequencer_frag_broadcast: Sender<FragMessage>,
    blockenv: Producer<InternalMessage<BlockEnv>>,
    timestamp: IngestionTime,
}

impl<Db: BopDbRead> From<&Spine<Db>> for SendersSpine<Db> {
    fn from(value: &Spine<Db>) -> Self {
        Self {
            sequencer_to_simulator: value.sender_sequencer_to_simulator.clone(),
            sequencer_to_rpc: value.sender_sequencer_to_rpc.clone(),
            simulator_to_sequencer: value.sender_simulator_to_sequencer.clone(),
            engine_rpc_to_sequencer: value.sender_engine_rpc_to_sequencer.clone(),
            eth_rpc_to_sequencer: value.sender_eth_rpc_to_sequencer.clone(),
            blockfetch_to_sequencer: value.sender_blockfetch_to_sequencer.clone(),
            sequencer_frag_broadcast: value.sender_sequencer_frag_broadcast.clone(),
            blockenv: value.blockenv.clone().into(),
            timestamp: Default::default(),
        }
    }
}

impl<Db: BopDbRead> TrackedSenders for SendersSpine<Db> {
    fn set_ingestion_t(&mut self, ingestion_t: IngestionTime) {
        self.timestamp = ingestion_t;
    }

    fn ingestion_t(&self) -> IngestionTime {
        self.timestamp
    }
}

#[derive(Debug)]
pub struct ReceiversSpine<Db: BopDbRead> {
    simulator_to_sequencer: Receiver<SimulatorToSequencer<Db>>,
    sequencer_to_simulator: Receiver<SequencerToSimulator<Db>>,
    sequencer_to_rpc: Receiver<SequencerToExternal>,
    engine_rpc_to_sequencer: Receiver<messages::EngineApi>,
    eth_rpc_to_sequencer: Receiver<Arc<Transaction>>,
    blockfetch_to_sequencer: Receiver<BlockSyncMessage>,
    sequencer_frag_broadcast: Receiver<FragMessage>,
    blockenv: Receiver<BlockEnv, Consumer<InternalMessage<BlockEnv>>>,
}

impl<Db: BopDbRead> ReceiversSpine<Db> {
    pub fn attach<S: AsRef<str>>(system_name: S, spine: &Spine<Db>) -> Self {
        Self {
            simulator_to_sequencer: Receiver::new(system_name.as_ref(), spine.into()),
            sequencer_to_simulator: Receiver::new(system_name.as_ref(), spine.into()),
            engine_rpc_to_sequencer: Receiver::new(system_name.as_ref(), spine.into()),
            eth_rpc_to_sequencer: Receiver::new(system_name.as_ref(), spine.into()),
            sequencer_to_rpc: Receiver::new(system_name.as_ref(), spine.into()),
            blockfetch_to_sequencer: Receiver::new(system_name.as_ref(), spine.into()),
            sequencer_frag_broadcast: Receiver::new(system_name.as_ref(), spine.into()),
            blockenv: Receiver::new(system_name.as_ref(), spine.blockenv.clone().into()),
        }
    }
}

pub type SpineConnections<Db> = Connections<SendersSpine<Db>, ReceiversSpine<Db>>;

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
