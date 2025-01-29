use bop_common::{
    actor::Actor,
    communication::{
        messages::{self, SequencerToSimulator, SimulatorToSequencer},
        sequencer::{ReceiversSequencer, SendersSequencer},
        simulator::{ReceiversSimulator, SendersSimulator},
        Connections, Spine, TrackedSenders,
    },
    config::Config,
    time::{Duration, Repeater},
    utils::{init_tracing, last_part_of_typename, wait_for_signal},
};
use bop_pool::transaction::pool::TxPool;
use revm_primitives::db::DatabaseRef;
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
        connections.receive(|msg, senders| match msg {
            SequencerToSimulator::SenderTxs(txs) => {
                info!("got simulate request: SenderTxs");
                if let Err(e) = senders.send(SimulatorToSequencer::SenderTxsSimulated(txs)) {
                    tracing::error!("Couldn't send reply:{e} ")
                };
            }
            SequencerToSimulator::Ping => {
                info!("Received Ping from simulator, sending pong");
                if let Err(e) = senders.send(SimulatorToSequencer::Pong(self.0)) {
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

#[allow(dead_code)]
pub struct Sequencer<Db: DatabaseRef> {
    tx_pool: TxPool,
    db: Db,
    every_s: Repeater,
}

impl<Db: DatabaseRef> Sequencer<Db> {
    fn new(db: Db) -> Self {
        Self { db, every_s: Repeater::every(Duration::from_secs(1)), tx_pool: TxPool::default() }
    }
}

const DEFAULT_BASE_FEE: u64 = 10;

impl<Db> Actor for Sequencer<Db>
where
    Db: DatabaseRef + Send,
    <Db as DatabaseRef>::Error: std::fmt::Debug,
{
    type Receivers = ReceiversSequencer;
    type Senders = SendersSequencer;

    const CORE_AFFINITY: Option<usize> = Some(0);

    fn on_init(&mut self, _connections: &mut Connections<SendersSequencer, ReceiversSequencer>) {
        info!("Demo init");
    }

    fn loop_body(&mut self, connections: &mut Connections<SendersSequencer, ReceiversSequencer>) {
        connections.receive(|msg: SimulatorToSequencer, _| {
            match msg {
                SimulatorToSequencer::SenderTxsSimulated(_) => {
                    info!("Received simulated txs from the simulator")
                }
                SimulatorToSequencer::Pong(i) => info!("Received pong from simulator {i}"),
            };
        });

        connections.receive(|msg: messages::EthApi, senders| {
            info!("received msg from ethapi");
            self.tx_pool.handle_new_tx(msg.order, &self.db, DEFAULT_BASE_FEE, senders);
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
    let _guards = init_tracing(Some("gateway"), 100, None);

    let spine = Spine::default();

    let rpc_config = Config::default();

    let db = bop_db::DbStub::default();

    std::thread::scope(|s| {
        s.spawn(|| {
            let rt = tokio::runtime::Builder::new_current_thread()
                .worker_threads(10)
                .enable_all()
                .build()
                .expect("failed to create runtime");
            let engine_server = bop_rpc::EngineRpcServer::new(&spine, rpc_config.api_timeout);
            rt.spawn(engine_server.run(rpc_config.engine_api_addr));
            let eth_server = bop_rpc::EthRpcServer::new(&spine, rpc_config.api_timeout);
            rt.spawn(eth_server.run(rpc_config.eth_api_addr));
            rt.block_on(wait_for_signal())
        });
        let sim_0 = Simulator(0);
        sim_0.run(s, &spine, Some(Duration::from_micros(100)), Some(1));
        let sim_1 = Simulator(1);
        sim_1.run(s, &spine, Some(Duration::from_micros(100)), Some(2));
        let sim_2 = Simulator(2);
        // Ok to also run on 1 as it is sleeping for quite some time if there's no work to be done
        sim_2.run(s, &spine, Some(Duration::from_micros(100)), Some(1));

        let sequencer = Sequencer::new(db);
        sequencer.run(s, &spine, None, Some(3));
    });
}
