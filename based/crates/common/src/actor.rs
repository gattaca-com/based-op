use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use core_affinity::CoreId;
use tracing::{info, span, warn, Level};

use crate::{
    communication::SpineConnections, db::{BopDbRead, DBFrag}, time::{vsync, Duration, Timer}, utils::last_part_of_typename
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
}

pub trait Actor<Db: BopDbRead>: Sized {
    const CORE_AFFINITY: Option<usize> = None;

    fn loop_body(&mut self, _connections: &mut SpineConnections<Db>) {}
    fn on_init(&mut self, _connections: &mut SpineConnections<Db>) {}
    fn on_exit(self, _connections: &mut SpineConnections<Db>) {}

    fn run(mut self, mut connections: SpineConnections<Db>, actor_config: ActorConfig) {
        let name = last_part_of_typename::<Self>();
        let _s = span!(Level::INFO, "", system = last_part_of_typename::<Self>()).entered();
        //TODO: Verify that this doesn't add too much time to the loop
        let term = Arc::new(AtomicBool::new(false));
        signal_hook::flag::register(signal_hook::consts::SIGINT, Arc::clone(&term))
            .expect("Couldn't register signal hook for some reason");

        if let Some(id) = actor_config.core.or(Self::CORE_AFFINITY) {
            if !core_affinity::set_for_current(CoreId { id }) {
                warn!("Couldn't set core_affinity");
            };
        }
        info!("Initializing...");
        self.on_init(&mut connections);
        info!("Initialized...");

        let mut loop_timer = Timer::new(format!("{}-loop", name));
        let min_loop_duration = actor_config.min_loop_duration;
        loop {
            loop_timer.start();
            if vsync(min_loop_duration, || {
                self.loop_body(&mut connections);
                loop_timer.stop();
                term.load(Ordering::Relaxed)
            }) {
                break;
            }
        }
        info!("Running final tasks before stopping...");
        self.on_exit(&mut connections);
        info!("Finalized");
    }
}
