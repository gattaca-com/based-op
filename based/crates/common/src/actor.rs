use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use core_affinity::CoreId;
use tracing::{info, span, warn, Level};

use crate::{
    communication::SpineConnections,
    db::BopDbRead,
    time::{vsync, Duration, Timer},
    utils::last_part_of_typename,
};

#[derive(Copy, Clone, Default)]
pub struct ActorConfig {
    /// To override default CORE_AFFINITY
    core: Option<usize>,

    /// If Some: every round through the Actor::loop_body will at least take this long.
    /// If work finished before this minimum time, the thread will sleep for the remainder.
    min_loop_duration: Option<Duration>,
}

impl ActorConfig {
    pub fn with_core(mut self, core: usize) -> Self {
        self.core = Some(core);
        self
    }

    pub fn with_min_loop_duration(mut self, min_loop_duration: Duration) -> Self {
        self.min_loop_duration = Some(min_loop_duration);
        self
    }

    pub fn maybe_bind_to_core(&self, fallback: Option<usize>) {
        if let Some(id) = self.core.or(fallback) {
            if !core_affinity::set_for_current(CoreId { id }) {
                warn!("Couldn't set core_affinity");
            };
        }
    }
}

pub trait Actor<Db: BopDbRead>: Sized {
    const CORE_AFFINITY: Option<usize> = None;
    fn loop_body(&mut self, _connections: &mut SpineConnections<Db>) {}
    fn on_init(&mut self, _connections: &mut SpineConnections<Db>) {}

    fn _on_init(&mut self, connections: &mut SpineConnections<Db>) {
        info!("Initializing...");
        self.on_init(connections);
        info!("Initialized...");
    }

    fn on_exit(self, _connections: &mut SpineConnections<Db>) {}
    fn _on_exit(self, connections: &mut SpineConnections<Db>) {
        info!("Running final tasks before stopping...");
        self.on_exit(connections);
        info!("Finalized");
    }

    fn run(mut self, mut connections: SpineConnections<Db>, actor_config: ActorConfig) {
        let name = last_part_of_typename::<Self>();

        let _s = span!(Level::INFO, "", system = name).entered();

        actor_config.maybe_bind_to_core(Self::CORE_AFFINITY);

        self._on_init(&mut connections);

        let mut loop_timer = Timer::new(format!("{}-loop", name));

        let term = Arc::new(AtomicBool::new(false));
        signal_hook::flag::register(signal_hook::consts::SIGINT, Arc::clone(&term))
            .expect("Couldn't register signal hook for some reason");

        loop {
            loop_timer.start();
            if vsync(actor_config.min_loop_duration, || {
                self.loop_body(&mut connections);
                loop_timer.stop();
                term.load(Ordering::Relaxed)
            }) {
                break;
            }
        }

        self._on_exit(&mut connections)
    }
}
