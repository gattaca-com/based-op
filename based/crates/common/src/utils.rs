use tokio::signal::unix::{signal, SignalKind};
use tracing::level_filters::LevelFilter;
use tracing_appender::{non_blocking::WorkerGuard, rolling::Rotation};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Layer};

pub const DEFAULT_TRACING_ENV_FILTERS: &[&str] = &[
    "hyper::proto::h1=off",
    "trust_dns_proto=off",
    "trust_dns_resolver=off",
    "discv5=off",
    "hyper_util=off",
    "reqwest=info",
    "jsonrpsee=info",
    "alloy_transport_http=info",
    "alloy_rpc_client=info",
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
        .with_thread_ids(false)
        .with_target(false)
        .with_timer(tracing_subscriber::fmt::time());

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

pub fn initialize_test_tracing(level: LevelFilter) {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_max_level(level)
        .with_thread_names(true)
        .with_file(true)
        .with_line_number(true)
        .init();
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

/// Strips all top-level (non-generic) path segments and returns only the last segment,
/// including any generic parameters.
///
/// For example:
/// - `foo::Bar::Baz<T::b::db>` => `Baz<T::b::db>`
/// - `my_crate::some_mod::Type<U>` => `Type<U>`
/// - `JustType` => `JustType`
///
/// If there are no top-level "::" separators, the original string is returned.
pub fn strip_namespace(s: &str) -> &str {
    let mut depth = 0;
    let mut start = 0;
    let bytes = s.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        match bytes[i] {
            b'<' => {
                // Entering a generic parameter region, increase depth
                depth += 1;
            }
            b'>' => {
                // Leaving a generic parameter region, decrease depth
                if depth > 0 {
                    depth -= 1;
                }
            }
            b':' => {
                // Check if we have "::" at top-level (depth == 0)
                if depth == 0 && i + 1 < len && bytes[i + 1] == b':' {
                    // Update start to skip past this "::"
                    start = i + 2;
                    // Skip the extra ':' in the loop
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }

    &s[start..]
}

pub fn last_part_of_typename<T>() -> &'static str {
    let full_name = strip_namespace(std::any::type_name::<T>());
    if let Some(generic_start) = full_name.find('<') {
        &full_name[..generic_start]
    } else {
        full_name
    }
}

pub fn last_part_of_typename_without_generic<T>() -> &'static str {
    let name = strip_namespace(std::any::type_name::<T>());
    if let Some(id) = name.find('<') {
        &name[..id]
    } else {
        name
    }
}

pub fn typename_no_generics<T>() -> &'static str {
    let name = strip_namespace(std::any::type_name::<T>());
    if let Some(last) = name.find('<') {
        &name[..last]
    } else {
        name
    }
}
