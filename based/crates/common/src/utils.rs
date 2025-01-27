use tokio::signal::unix::{signal, SignalKind};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

pub fn init_tracing() -> WorkerGuard {
    let (non_blocking, guard) = tracing_appender::non_blocking(std::io::stdout());
    tracing_subscriber::registry()
        .with(EnvFilter::from_default_env())
        .with(fmt::layer().with_writer(non_blocking))
        .init();

    guard
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

/// foo::bar::Baz<T> -> Baz<T>
pub fn after_last_colons(name: &str) -> &str {
    let name = if let Some(colon) = name.rfind("::") { &name[colon + 2..] } else { name };
    if let Some(end_caret) = name.rfind('>') {
        &name[..=end_caret]
    } else {
        name
    }
}

pub fn last_part_of_typename<T>() -> &'static str {
    after_last_colons(std::any::type_name::<T>())
}
