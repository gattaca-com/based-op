use std::{fs::read_dir, path::Path};

use shared_memory::ShmemError;
use thiserror::Error;

pub mod queue;
pub mod seqlock;
pub use queue::{Consumer, Producer, Queue};
pub use seqlock::Seqlock;
pub mod internal_message;

pub use internal_message::InternalMessage;

pub type Sender<T> = crossbeam_channel::Sender<InternalMessage<T>>;
pub type Receiver<T> = crossbeam_channel::Sender<InternalMessage<T>>;

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
    let queues_dir =
        directories::BaseDirs::new().expect("Couldn't retrieve home dir").data_dir().join("builder/queues");
    queues_dir.to_string_lossy().to_string()
}

pub fn verify_or_remove_queue_files() {
    let queues_dir =
        directories::BaseDirs::new().expect("Couldn't retrieve home dir").data_dir().join("builder/queues");
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
