use std::sync::Arc;

use bop_common::{
    actor::Actor,
    communication::{messages::SequencerToSimulator, Connections, ReceiversSpine, SendersSpine, TrackedSenders},
    time::Duration,
    transaction::Transaction,
    utils::last_part_of_typename,
};
use bop_db::BopDB;
//use revm::db::CacheDB;
use revm_primitives::BlockEnv;
use tracing::info;

//type CacheDBPartiallyBuilt<Db> = CacheDB<Arc<CacheDB<Db>>>;

#[derive(Clone, Default)]
pub struct Simulator<Db> {
    id: usize,
    // evm: Option<Evm<'static, AddressScreener, CacheDBPartiallyBuilt<Db>>>,
    _block_env: BlockEnv,
    _o: Option<Db>, /* spec_id: OpChainSpec,
                     * For use in sort requests
                     * evm_partially_built: Evm<'static, AddressScreener, >, */
}

impl<Db: BopDB> Simulator<Db> {
    pub fn new(id: usize) -> Self {
        // let evm_tob = Evm::builder().with_db(CacheDB::new(db)).with_env(env).with_spec_id(spec_id).build();

        // let evm_tob =
        // Self { id, evm: None, block_env: BlockEnv::default(), spec_id: OpChainSpec::default_mainnet() }
        Self { id, _block_env: BlockEnv::default(), _o: None }
    }

    fn simulate_tx_list(&self, tx_list: Vec<Arc<Transaction>>) -> Vec<Arc<Transaction>> {
        tx_list
    }

    // fn evm(&mut self, db: CacheDB<Arc<CacheDB<Db>>>) -> &mut Evm<'static, AddressScreener, CacheDBPartiallyBuilt<Db>>
    // {     self.evm.get_or_insert_with(|| {})
    // }
}

impl<Db: BopDB> Actor<Db> for Simulator<Db> {
    const CORE_AFFINITY: Option<usize> = None;

    fn name(&self) -> String {
        format!("{}-{}", last_part_of_typename::<Self>(), self.id)
    }

    fn loop_body(&mut self, connections: &mut Connections<SendersSpine<Db>, ReceiversSpine<Db>>) {
        connections.receive(|msg: SequencerToSimulator<Db>, senders| {
            info!("received {}", msg.as_ref());
            match msg {
                SequencerToSimulator::SimulateTxList(_db, txs) => {
                    debug_assert!(
                        senders
                            .send_timeout(
                                bop_common::communication::messages::SimulatorToSequencer::SimulatedTxList(
                                    self.simulate_tx_list(txs)
                                ),
                                Duration::from_millis(10)
                            )
                            .is_ok(),
                        "timed out trying to send request"
                    );
                }
                SequencerToSimulator::NewBlock => {
                    todo!()
                }
            }
        });
    }
}
