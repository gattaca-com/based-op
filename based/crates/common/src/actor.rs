use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use tracing::{debug, span, Level};

use crate::{
    communication::SpineConnections,
    time::{vsync, Duration, Timer},
    utils::last_part_of_typename,
};

#[derive(Copy, Clone, Default)]
pub struct ActorConfig {
    /// If Some: every round through the Actor::loop_body will at least take this long.
    /// If work finished before this minimum time, the thread will sleep for the remainder.
    min_loop_duration: Option<Duration>,
}

impl ActorConfig {
    pub fn with_min_loop_duration(mut self, min_loop_duration: Duration) -> Self {
        self.min_loop_duration = Some(min_loop_duration);
        self
    }
}

pub trait Actor<Db>: Sized {
    fn name(&self) -> String {
        last_part_of_typename::<Self>().to_string()
    }

    fn loop_body(&mut self, _connections: &mut SpineConnections<Db>) {}
    fn on_init(&mut self, _connections: &mut SpineConnections<Db>) {}

    fn _on_init(&mut self, connections: &mut SpineConnections<Db>) {
        debug!("initializing...");
        self.on_init(connections);
        debug!("initialized...");
    }

    fn on_exit(self, _connections: &mut SpineConnections<Db>) {}
    fn _on_exit(self, connections: &mut SpineConnections<Db>) {
        debug!("running final tasks before stopping...");
        self.on_exit(connections);
        debug!("finalized");
    }

    fn run(mut self, mut connections: SpineConnections<Db>, actor_config: ActorConfig) {
        let name = self.name();
        let _s = span!(Level::INFO, "", id = name).entered();

        let mut loop_timer = Timer::new(format!("{}-loop", name));

        self._on_init(&mut connections);

        let term = Arc::new(AtomicBool::new(false));
        signal_hook::flag::register(signal_hook::consts::SIGINT, Arc::clone(&term))
            .expect("couldn't register signal hook for some reason");

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
