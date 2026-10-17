use std::path::Path;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{Builder, Rotation};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, fmt};

/// Days of rotated log files to keep.
const MAX_LOG_FILES: usize = 14;
const DEFAULT_FILTER: &str = "info,rc_riot=debug,rc_jobs=debug,rift_coach=debug";

/// Keeps the background log writer alive; drop it on shutdown to flush.
pub struct LogGuard(#[allow(dead_code)] WorkerGuard);

/// Structured JSON logs to a daily-rotating file, plus human-readable stderr.
/// Filter with `RUST_LOG` (e.g. `RUST_LOG=rc_riot=trace`).
pub fn init(logs_dir: &Path, file_prefix: &str) -> std::io::Result<LogGuard> {
    let appender = Builder::new()
        .rotation(Rotation::DAILY)
        .filename_prefix(file_prefix)
        .filename_suffix("log")
        .max_log_files(MAX_LOG_FILES)
        .build(logs_dir)
        .map_err(std::io::Error::other)?;
    let (writer, guard) = tracing_appender::non_blocking(appender);

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(DEFAULT_FILTER));
    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().json().with_current_span(false).with_writer(writer))
        .with(fmt::layer().compact().with_writer(std::io::stderr))
        .try_init()
        .map_err(std::io::Error::other)?;
    Ok(LogGuard(guard))
}
