use tokio::signal::unix::{signal, SignalKind};
use tracing::level_filters::LevelFilter;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, Layer, Registry};

pub fn init_tracing(log_file: bool) -> WorkerGuard {
    let format = tracing_subscriber::fmt::format()
        .with_level(true)
        .with_thread_ids(true)
        .with_target(false)
        .with_timer(tracing_subscriber::fmt::time())
        .compact();

    let stdout_log = tracing_subscriber::fmt::layer()
        .event_format(format.clone())
        .with_filter(tracing_subscriber::EnvFilter::from_default_env());
    let subscriber = Registry::default().with(stdout_log);

    let file_log = if log_file {
        let path = "log.log";

        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .expect("couldn't create log file");
        Some(
            tracing_subscriber::fmt::layer()
                .event_format(format)
                .with_writer(file)
                .with_filter(LevelFilter::DEBUG),
        )
    } else {
        None
    };
    let (non_blocking, guard) = tracing_appender::non_blocking(std::io::stdout());
    subscriber.with(file_log).with(fmt::layer().with_writer(non_blocking)).init();
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
