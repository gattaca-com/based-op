#[cfg(not(test))]
fn main() {}

#[cfg(test)]
mod db;
#[cfg(test)]
mod rpc;
#[cfg(test)]
mod bop_tests {

    use std::process::Command;

    use alloy_primitives::U256;
    use bop_common::actor::{Actor, ActorConfig};
    use bop_db::BopDB;
    use bop_sequencer::SequencerConfig;

    #[test]
    fn db() {
        let db = super::db::GenesisDB::new_with_randoms(10);
        assert_eq!(db.len(), 10);
        assert_eq!(db.balance(db.rand_addr()), Some(U256::from_limbs([123, 0, 0, 0])));
    }

    #[test]
    fn sequencer() {
        let db_bop = super::db::GenesisDB::new_with_randoms(10);
        let spine = bop_common::communication::Spine::default();
        let db_read: bop_common::db::DBFrag<_> = db_bop.readonly().expect("Failed to create read-only DB").into();
        std::thread::scope(|s| {
            let rt = tokio::runtime::Builder::new_current_thread()
                .worker_threads(1)
                .enable_all()
                .build()
                .expect("failed to create runtime")
                .into();

            s.spawn(|| {
                let sequencer = bop_sequencer::Sequencer::new(db_bop, db_read, rt, SequencerConfig::default());
                let connections = spine.to_connections("sequencer");
                sequencer.run(connections, ActorConfig::default())
            });



            std::thread::sleep(std::time::Duration::from_secs(10));

            let mut kill = Command::new("kill")
                .arg("-s")
                .arg("SIGTERM")
                .arg(std::process::id().to_string())
                .spawn()
                .expect("issue killing");

            kill.wait().expect("couldn't kill bop_tests");
        });

    }
}
