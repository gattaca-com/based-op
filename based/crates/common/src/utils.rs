use std::time::{SystemTime, UNIX_EPOCH};

use tokio::signal::unix::{signal, SignalKind};
use tracing::Level;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

/// Seconds
pub fn utcnow_sec() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
}
/// Millis
pub fn utcnow_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64
}
/// Micros
pub fn utcnow_us() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_micros() as u64
}
/// Nanos
pub fn utcnow_ns() -> u64 {
    // safe until ~2554
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos() as u64
}

pub fn init_tracing(level: Option<Level>) -> WorkerGuard {
    let (non_blocking, guard) = tracing_appender::non_blocking(std::io::stdout());
    let registry = tracing_subscriber::registry();

    let registry = if let Some(level) = level {
        registry.with(EnvFilter::new(level.as_str()))
    } else {
        registry.with(EnvFilter::from_default_env())
    };

    registry.with(fmt::layer().with_writer(non_blocking)).init();

    guard
}

pub fn initialize_test_tracing() {
    tracing_subscriber::fmt().with_max_level(tracing::Level::DEBUG).init();
}

pub async fn wait_for_signal() -> eyre::Result<()> {
    let mut sigint = signal(SignalKind::interrupt())?;
    let mut sigterm = signal(SignalKind::terminate())?;

    tokio::select! {
        _ = sigint.recv() => {}
        _ = sigterm.recv() => {}
    }

    Ok(())
}
