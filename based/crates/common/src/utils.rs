use tokio::signal::unix::{signal, SignalKind};
use tracing::{error, level_filters::LevelFilter};
use tracing_appender::{non_blocking::WorkerGuard, rolling::Rotation};
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Layer, Registry};

pub const DEFAULT_TRACING_ENV_FILTERS: [&str; 6] = [
    "hyper::proto::h1=off",
    "trust_dns_proto=off",
    "trust_dns_resolver=off",
    "discv5=off",
    "hyper_util=off",
    "reqwest=info",
];

/// Builds an environment filter for logging. Uses a default set of filters plus some optional
/// extras.
pub fn build_env_filter(env_filters: Option<Vec<&str>>) -> EnvFilter {
    let mut env_filter = EnvFilter::builder().from_env_lossy();

    for directive in DEFAULT_TRACING_ENV_FILTERS {
        env_filter = env_filter.add_directive(directive.parse().unwrap());
    }

    if let Some(env_filters) = env_filters {
        for directive in env_filters {
            if !DEFAULT_TRACING_ENV_FILTERS.contains(&directive) {
                env_filter = env_filter.add_directive(directive.parse().unwrap());
            }
        }
    }

    env_filter
}

/// Initialises tracing logger that creates daily log files.
pub fn init_tracing(
    filename_prefix: Option<&str>,
    max_log_files: usize,
    env_filters: Option<Vec<&str>>,
) -> (Option<WorkerGuard>, WorkerGuard) {
    let format = tracing_subscriber::fmt::format()
        .with_level(true)
        .with_thread_ids(true)
        .with_target(false)
        .with_timer(tracing_subscriber::fmt::time())
        .compact();

    let (file_layer, worker_guard) = if let Some(fname) = filename_prefix {
        let log_path = std::env::var("LOG_PATH").unwrap_or("/tmp".into());

        let file_appender = tracing_appender::rolling::Builder::new()
            .filename_prefix(fname)
            .max_log_files(max_log_files)
            .rotation(Rotation::DAILY)
            .build(log_path)
            .expect("failed to create log appender!");
        let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
        (
            Some(
                tracing_subscriber::fmt::layer()
                    .event_format(format.clone())
                    .with_writer(non_blocking)
                    .with_filter(build_env_filter(env_filters.clone())),
            ),
            Some(guard),
        )
    } else {
        (None, None)
    };
    let (stdout_writer, stdout_guard) = tracing_appender::non_blocking(std::io::stdout());
    let stdout_layer = tracing_subscriber::fmt::layer()
        .event_format(format.clone())
        .with_writer(stdout_writer)
        .with_filter(build_env_filter(env_filters));
    tracing_subscriber::registry().with(stdout_layer).with(file_layer).init();
    (worker_guard, stdout_guard)
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
