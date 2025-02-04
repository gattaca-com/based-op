use std::{
    future::Future,
    sync::{Arc, OnceLock},
};

use tokio::{
    runtime::{Handle, Runtime},
    task::JoinHandle,
};

static RUNTIME: OnceLock<Handle> = OnceLock::new();

pub fn init_runtime() {
    RUNTIME.get_or_init(build_tokio_runtime);
}

pub fn spawn<F>(future: F) -> JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    match RUNTIME.get() {
        Some(runtime) => runtime.spawn(future),
        None => panic!("runtime has not been initialised!"),
    }
}

pub fn build_tokio_runtime() -> Handle {
    if let Ok(handle) = Handle::try_current() {
        return handle;
    }

    tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("failed to create runtime").handle().clone()
}

/// A wrapper around a runtime or handle.
///
/// If spawning from sync context, use the Runtime variant.
/// If spawning from async context, use the Handle variant.
#[derive(Debug, Clone)]
pub enum RuntimeOrHandle {
    Runtime(Arc<Runtime>),
    Handle(Handle),
}

impl RuntimeOrHandle {
    pub fn new_runtime(runtime: Arc<Runtime>) -> Self {
        Self::Runtime(runtime)
    }

    pub fn new_handle(handle: Handle) -> Self {
        Self::Handle(handle)
    }

    pub fn spawn<F>(&self, future: F) -> JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        match self {
            Self::Runtime(runtime) => runtime.spawn(future),
            Self::Handle(handle) => handle.spawn(future),
        }
    }

    pub fn block_on<F>(&self, future: F) -> F::Output
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        match self {
            Self::Runtime(runtime) => runtime.block_on(future),
            Self::Handle(handle) => handle.block_on(future),
        }
    }
}

impl From<Handle> for RuntimeOrHandle {
    fn from(handle: Handle) -> Self {
        Self::Handle(handle)
    }
}

impl From<Arc<Runtime>> for RuntimeOrHandle {
    fn from(runtime: Arc<Runtime>) -> Self {
        Self::Runtime(runtime)
    }
}
