use std::{fs, io, path::Path, sync::Arc};

use reth_chainspec::ChainSpecBuilder;
use reth_db::{init_db, ClientVersion};
use reth_provider::{providers::StaticFileProvider, ProviderFactory};
use reth_storage_errors::db::LogLevel;

use super::{Error, DB};

/// Initialise the database.
/// # Params
/// * `db_location` - disk location of the directory holding the database files. This must be the top level directory
///   containing `db` and `static_files` subdirectories. The directories will be created if they do not exist.
///
/// Returns the initialised [`BopDB`] implementation, or [`Error`] if there is a problem.
pub fn init_database<P: AsRef<Path>>(db_location: P) -> Result<DB, Error> {
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
        .with_exclusive(Some(false));
    let db = Arc::new(init_db(db_dir, db_args).map_err(|e| Error::DatabaseInitialisationError(e.to_string()))?);

    let chain_spec = Arc::new(ChainSpecBuilder::mainnet().build()); // TODO
    let provider = ProviderFactory::new(db, chain_spec, StaticFileProvider::read_write(static_files_dir)?);
    Ok(DB { provider })
}

fn create_or_check_dir<P: AsRef<Path>>(dir: &P) -> Result<(), Error> {
    if fs::exists(dir).map_err(|e| Error::DirNotReadable(path_string(dir), e))? {
        let dir_metadata = fs::metadata(dir).map_err(|e| Error::DirNotReadable(path_string(dir), e))?;
        if dir_metadata.permissions().readonly() {
            tracing::error!(dir = path_string(dir), "Database directory is read-only");
            return Err(Error::DirNotWritable(path_string(dir), io::ErrorKind::PermissionDenied.into()));
        }
    } else {
        fs::create_dir_all(dir).map_err(|e| Error::DirNotWritable(path_string(dir), e))?;
    }

    Ok(())
}

fn path_string<P: AsRef<Path>>(dir: &P) -> String {
    dir.as_ref().to_str().unwrap_or("<unknown>").to_string()
}
