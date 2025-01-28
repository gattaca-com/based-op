use bop_common::{
    actor::Actor,
    communication::{
        Connections, ReceiversSequencer, ReceiversSimulator, SendersSequencer, SendersSimulator, SequencerToSimulator,
        SimulatorToSequencer, Spine, TrackedSenders,
    },
    time::{Duration, Repeater},
    utils::{init_tracing, last_part_of_typename},
};
use tracing::{error, info};

pub struct Simulator(usize);

impl Actor for Simulator {
    type Receivers = ReceiversSimulator;
    type Senders = SendersSimulator;

    const CORE_AFFINITY: Option<usize> = None;

    fn on_init(&mut self, _connections: &mut Connections<SendersSimulator, ReceiversSimulator>) {
        info!("Demo init");
    }

    fn name(&self) -> String {
        format!("{}-{}", last_part_of_typename::<Self>(), self.0)
    }

    fn loop_body(&mut self, connections: &mut Connections<SendersSimulator, ReceiversSimulator>) {
        connections.receive(|msg, producers| match msg {
            SequencerToSimulator::Ping => {
                info!("Received Ping from simulator, sending pong");
                if let Err(e) = producers.send(SimulatorToSequencer::Pong(self.0)) {
                    error!("Issue sending pong from sim to sequencer: {e}");
                }
            }
        });
    }

    fn on_exit(self, _connections: &mut Connections<SendersSimulator, ReceiversSimulator>) {
        info!("Demo final tasks");
    }

    fn create_senders(&self, spine: &Spine) -> Self::Senders {
        spine.into()
    }

    fn create_receivers(&self, spine: &Spine) -> Self::Receivers {
        Self::Receivers::new(self, spine)
    }
}

pub struct Sequencer {
    every_s: Repeater,
}

impl Default for Sequencer {
    fn default() -> Self {
        Self { every_s: Repeater::every(Duration::from_secs(1)) }
    }
}

impl Actor for Sequencer {
    type Receivers = ReceiversSequencer;
    type Senders = SendersSequencer;

    const CORE_AFFINITY: Option<usize> = Some(0);

    fn on_init(&mut self, _connections: &mut Connections<SendersSequencer, ReceiversSequencer>) {
        info!("Demo init");
    }

    fn loop_body(&mut self, connections: &mut Connections<SendersSequencer, ReceiversSequencer>) {
        connections.receive(|msg: SimulatorToSequencer, _| {
            match msg {
                SimulatorToSequencer::Pong(i) => info!("Received pong from simulator {i}"),
            };
        });
        if self.every_s.fired() {
            info!("sending 4 pings");
            for _ in 0..4 {
                if let Err(e) = connections.send(SequencerToSimulator::Ping) {
                    error!("issue sending ping to simulator: {e}");
                }
            }
        }
    }

    fn on_exit(self, _connections: &mut Connections<SendersSequencer, ReceiversSequencer>) {
        info!("Demo final tasks");
    }

    fn create_senders(&self, spine: &Spine) -> Self::Senders {
        spine.into()
    }

    fn create_receivers(&self, spine: &Spine) -> Self::Receivers {
        Self::Receivers::new(self, spine)
    }
}

fn main() {
    let _guard = init_tracing(true);
    let spine = Spine::default();

    std::thread::scope(|s| {
        let sim_0 = Simulator(0);
        sim_0.run(s, &spine, Some(Duration::from_micros(100)), Some(1));
        let sim_1 = Simulator(1);
        sim_1.run(s, &spine, Some(Duration::from_micros(100)), Some(2));
        let sim_2 = Simulator(2);
        // Ok to also run on 1 as it is sleeping for quite some time if there's no work to be done
        sim_2.run(s, &spine, Some(Duration::from_micros(100)), Some(1));

        let sequencer = Sequencer::default();
        sequencer.run(s, &spine, None, Some(3));
    });
}
