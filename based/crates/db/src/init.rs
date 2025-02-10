use std::{fs, path::Path, sync::Arc};

use eyre::bail;
use reth_db::{init_db, mdbx::MaxReadTransactionDuration, ClientVersion};
use reth_db_common::init::init_genesis;
use reth_optimism_chainspec::OpChainSpec;
use reth_provider::{providers::StaticFileProvider, ProviderFactory};
use reth_storage_errors::db::LogLevel;

use super::{Error, SequencerDB};

/// Initialise the database.
/// # Params
/// * `db_location` - disk location of the directory holding the database files. This must be the top level directory
///   containing `db` and `static_files` subdirectories. The directories will be created if they do not exist.
/// * `max_cached_accounts` - maximum number of `AccountInfo` structs to cache in database read caches.
/// * `max_cached_storages` - maximum number of individual storage slots to cache in database read caches.
///
/// Returns the initialised [`SequencerDB`] implementation, or [`Error`] if there is a problem.
pub fn init_database<P: AsRef<Path>>(
    db_location: P,
    max_cached_accounts: u64,
    max_cached_storages: u64,
    chain_spec: Arc<OpChainSpec>,
) -> eyre::Result<SequencerDB> {
    // Check the specified path is accessible, creating directories if necessary.
    let db_dir = db_location.as_ref().join("db");
    let static_files_dir = db_location.as_ref().join("static_files");
    let revert_files_dir = db_location.as_ref().join("reverts");
    create_or_check_dir(&db_dir)?;
    create_or_check_dir(&static_files_dir)?;
    create_or_check_dir(&revert_files_dir)?;

    let default_client_version =
        ClientVersion { version: "V1".into(), git_sha: "GITSHA1".into(), build_timestamp: "now".to_string() };
    let db_args = reth_db::mdbx::DatabaseArguments::new(default_client_version)
        .with_log_level(Some(LogLevel::Error))
        .with_max_read_transaction_duration(Some(MaxReadTransactionDuration::Unbounded))
        .with_exclusive(Some(false));
    let db = Arc::new(init_db(db_dir, db_args).map_err(|e| Error::DatabaseInitialisationError(e.to_string()))?);
    let factory = ProviderFactory::new(db, chain_spec.clone(), StaticFileProvider::read_write(static_files_dir)?);

    init_genesis(&factory)?;

    let db = SequencerDB::new(factory, max_cached_accounts, max_cached_storages);
    // check_init_genesis(&db, &chain_spec)?;

    Ok(db)
}

fn create_or_check_dir<P: AsRef<Path>>(dir: &P) -> eyre::Result<()> {
    if fs::exists(dir).map_err(|e| Error::DirNotReadable(path_string(dir), e))? {
        let test_file = dir.as_ref().join("ACCESS_CHECK");
        match fs::File::create(&test_file) {
            Ok(_) => {
                let _ = fs::remove_file(&test_file);
            }
            Err(_) => {
                bail!("Database directory is read-only, dir: {}", path_string(dir));
            }
        }
    } else {
        fs::create_dir_all(dir).map_err(|e| Error::DirNotWritable(path_string(dir), e))?;
    }

    Ok(())
}

fn path_string<P: AsRef<Path>>(dir: &P) -> String {
    dir.as_ref().to_str().unwrap_or("<unknown>").to_string()
}

// /// Initialize genesis state if needed
// /// Adapted from [`reth_db_common::init::init_genesis`]
// fn check_init_genesis(db: &SequencerDB, chain: &Arc<OpChainSpec>) -> eyre::Result<()> {
//     info!("checking init genesis");

//     let genesis = chain.genesis();

//     if let Ok(genesis_hash) = db.block_hash_ref(0) {
//         ensure!(
//             genesis_hash == chain.genesis_hash(),
//             "genesis hash mismatch, db: {:?}, chain_spec: {:?}",
//             genesis_hash,
//             chain.genesis_hash()
//         );
//     }

//     let head_block_number = db.head_block_number()?;
//     if head_block_number > 0 {
//         info!(head_block_number, "genesis already applied");
//         return Ok(());
//     }

//     // initialize the db

//     info!("applying genesis state");
//     let factory = &db.factory;

//     // TODO: we probably dont need some of these inserts

//     let alloc = &genesis.alloc;

//     // use transaction to insert genesis header
//     let provider_rw = factory.database_provider_rw()?;
//     insert_genesis_hashes(&provider_rw, alloc.iter())?;
//     insert_genesis_history(&provider_rw, alloc.iter())?;

//     // Insert header
//     insert_genesis_header(&provider_rw, &chain)?;

//     insert_genesis_state(&provider_rw, alloc.iter())?;

//     // insert sync stage
//     for stage in StageId::ALL {
//         provider_rw.save_stage_checkpoint(stage, Default::default())?;
//     }

//     let static_file_provider = provider_rw.static_file_provider();
//     // Static file segments start empty, so we need to initialize the genesis block.
//     let segment = StaticFileSegment::Receipts;
//     static_file_provider.latest_writer(segment)?.increment_block(0)?;

//     let segment = StaticFileSegment::Transactions;
//     static_file_provider.latest_writer(segment)?.increment_block(0)?;

//     // `commit_unwind`` will first commit the DB and then the static file provider, which is
//     // necessary on `init_genesis`.
//     UnifiedStorageWriter::commit_unwind(provider_rw)?;

//     Ok(())
// }
