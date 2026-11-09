// src-tauri/src/logger.rs
use std::path::Path;
use tracing_appender::non_blocking::WorkerGuard;

/// Initialises daily-rotating file logging to `log_dir/badger.log.YYYY-MM-DD`.
///
/// **The returned `WorkerGuard` MUST be stored for the app's full lifetime.**
/// Dropping it stops all log writes immediately and silently.
/// Store it via `app.manage(guard)` in the Tauri setup closure.
pub fn init_logger(log_dir: &Path) -> WorkerGuard {
    std::fs::create_dir_all(log_dir).expect("failed to create log directory");

    // daily() appends ".YYYY-MM-DD" to the filename prefix automatically
    let file_appender = tracing_appender::rolling::daily(log_dir, "badger.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    tracing_subscriber::fmt()
        .with_writer(non_blocking)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    guard
}
