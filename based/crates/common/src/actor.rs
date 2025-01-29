use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread::Scope,
};

use core_affinity::CoreId;
use tracing::{info, span, warn, Level};

use crate::{
    communication::{Connections, Spine},
    time::{vsync, Duration, Timer},
    utils::last_part_of_typename,
};

pub trait Actor: Send + Sized {
    const CORE_AFFINITY: Option<usize> = None;
    type Senders: Send;
    type Receivers: Send;

    fn create_senders<Db>(&self, spine: &Spine<Db>) -> Self::Senders;
    fn create_receivers<Db>(&self, spine: &Spine<Db>) -> Self::Receivers;

    fn loop_body(&mut self, _connections: &mut Connections<Self::Senders, Self::Receivers>) {}
    fn on_init(&mut self, _connections: &mut Connections<Self::Senders, Self::Receivers>) {}
    fn on_exit(self, _connections: &mut Connections<Self::Senders, Self::Receivers>) {}

    fn time_loop(&self) -> bool {
        true
    }

    fn name(&self) -> String {
        last_part_of_typename::<Self>().to_string()
    }

    fn run<'a, Db>(
        mut self,
        scope: &'a Scope<'a, '_>,
        spine: &Spine<Db>,
        min_loop_duration: Option<Duration>,
        affinity_override: Option<usize>,
    ) where
        Self: 'a,
    {
        let mut loop_timer = if self.time_loop() { Some(Timer::new(format!("{}-loop", self.name()))) } else { None };

        let mut connections = Connections::new(self.create_senders(spine), self.create_receivers(spine));
        scope.spawn(move || {
            let _s = span!(Level::INFO, "", system = last_part_of_typename::<Self>()).entered();
            //TODO: Verify that this doesn't add too much time to the loop
            let term = Arc::new(AtomicBool::new(false));
            signal_hook::flag::register(signal_hook::consts::SIGINT, Arc::clone(&term))
                .expect("Couldn't register signal hook for some reason");

            if let Some(id) = affinity_override.or(Self::CORE_AFFINITY) {
                if !core_affinity::set_for_current(CoreId { id }) {
                    warn!("Couldn't set core_affinity");
                };
            }
            info!("Initializing...");
            self.on_init(&mut connections);
            info!("Initialized...");

            loop {
                if let Some(t) = loop_timer.as_mut() {
                    t.start()
                };
                if vsync(min_loop_duration, || {
                    self.loop_body(&mut connections);
                    if let Some(t) = loop_timer.as_mut() {
                        t.stop()
                    };
                    term.load(Ordering::Relaxed)
                }) {
                    break;
                }
            }
            info!("Running final tasks before stopping...");
            self.on_exit(&mut connections);
            info!("Finalized");
        });
    }
}
