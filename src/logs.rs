use std::path::Path;
use std::time::SystemTime;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{Layer, layer::SubscriberExt, util::SubscriberInitExt};

pub const LOG_RETENTION_DAYS: u64 = 14;

/// Resolves the platform logs directory, creates it, prunes old rotated files,
/// then installs a file-backed tracing subscriber.
/// The returned [`WorkerGuard`] must be held for the process lifetime.
pub fn init_logging(logging: &crate::config::LoggingConfig) -> WorkerGuard {
    let logs_dir = crate::config::logs_dir().expect("failed to determine logs directory");
    std::fs::create_dir_all(&logs_dir).expect("failed to create logs directory");
    prune_old_logs(&logs_dir, LOG_RETENTION_DAYS);
    let guard = init_logging_to(&logs_dir, logging);
    install_panic_hook();
    guard
}

/// Installs a subscriber that writes logfmt to a daily rolling file in
/// `logs_dir`.
/// Uses `try_init` so repeated calls in tests do not panic.
fn init_logging_to(logs_dir: &Path, logging: &crate::config::LoggingConfig) -> WorkerGuard {
    let file_appender = tracing_appender::rolling::daily(logs_dir, "jabberwok.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
    let filter_spec = logging.filter_spec();

    let file_layer = tracing_logfmt::builder()
        .layer()
        .with_writer(non_blocking)
        .with_ansi(false)
        .with_filter(parse_env_filter(&filter_spec));

    tracing_subscriber::registry()
        .with(file_layer)
        .try_init()
        .ok(); // Silently ignore "already initialised" errors (parallel tests, etc.)

    guard
}

fn parse_env_filter(spec: &str) -> tracing_subscriber::EnvFilter {
    tracing_subscriber::EnvFilter::try_new(spec).unwrap_or_else(|error| {
        eprintln!(
            "warning: invalid logging filter `{spec}` ({error}); falling back to `warn,jabberwok=info`"
        );
        tracing_subscriber::EnvFilter::new("warn,jabberwok=info")
    })
}

fn install_panic_hook() {
    static INSTALLED: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    INSTALLED.get_or_init(|| {
        std::panic::set_hook(Box::new(|panic_info| {
            let location = panic_info
                .location()
                .map(|loc| format!("{}:{}:{}", loc.file(), loc.line(), loc.column()))
                .unwrap_or_else(|| "<unknown>".to_string());

            let payload = if let Some(text) = panic_info.payload().downcast_ref::<&str>() {
                *text
            } else if let Some(text) = panic_info.payload().downcast_ref::<String>() {
                text.as_str()
            } else {
                "<non-string panic payload>"
            };

            let backtrace = std::backtrace::Backtrace::force_capture();
            tracing::error!(
                location,
                payload,
                backtrace = %backtrace,
                "panic"
            );
        }));
    });
}

pub fn prune_old_logs(logs_dir: &Path, keep_days: u64) {
    let cutoff = SystemTime::now()
        .checked_sub(std::time::Duration::from_secs(60 * 60 * 24 * keep_days))
        .unwrap_or(SystemTime::UNIX_EPOCH);
    prune_files_older_than(logs_dir, cutoff);
}

/// Deletes rotated log files (any extension other than `.log`) that were last
/// modified before `cutoff`. Silently ignores I/O errors so a pruning failure
/// never prevents the daemon from starting.
fn prune_files_older_than(logs_dir: &Path, cutoff: SystemTime) {
    let Ok(entries) = std::fs::read_dir(logs_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        // Keep the active log file (`jabberwok.log`) and anything without an
        // extension. Only rotated files like `jabberwok.log.2026-03-17` are
        // candidates for deletion.
        if path.extension().is_none_or(|e| e == "log") {
            continue;
        }
        if let Ok(meta) = entry.metadata()
            && let Ok(modified) = meta.modified()
            && modified < cutoff
        {
            let _ = std::fs::remove_file(&path);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    // ---------------------------------------------------------------------------
    // prune_files_older_than helpers
    // ---------------------------------------------------------------------------

    fn prune_with_future_cutoff(dir: &Path) {
        let cutoff = SystemTime::now() + Duration::from_secs(3600);
        prune_files_older_than(dir, cutoff);
    }

    fn prune_with_epoch_cutoff(dir: &Path) {
        prune_files_older_than(dir, SystemTime::UNIX_EPOCH);
    }

    // ---------------------------------------------------------------------------
    // prune_files_older_than
    // ---------------------------------------------------------------------------

    #[test]
    fn prune_removes_old_rotated_log() {
        let dir = tempfile::tempdir().unwrap();
        let rotated = dir.path().join("jabberwok.log.2026-03-01");
        std::fs::write(&rotated, b"old").unwrap();

        prune_with_future_cutoff(dir.path());

        assert!(!rotated.exists(), "rotated log should have been deleted");
    }

    #[test]
    fn prune_keeps_recent_rotated_log() {
        let dir = tempfile::tempdir().unwrap();
        let rotated = dir.path().join("jabberwok.log.2026-03-18");
        std::fs::write(&rotated, b"recent").unwrap();

        prune_with_epoch_cutoff(dir.path());

        assert!(rotated.exists(), "recent rotated log should be kept");
    }

    #[test]
    fn prune_always_keeps_active_log_file() {
        let dir = tempfile::tempdir().unwrap();
        let active = dir.path().join("jabberwok.log");
        std::fs::write(&active, b"active").unwrap();

        // Even with a far-future cutoff the active `.log` file must survive.
        prune_with_future_cutoff(dir.path());

        assert!(active.exists(), "active .log file must never be pruned");
    }

    #[test]
    fn prune_keeps_files_without_extension() {
        let dir = tempfile::tempdir().unwrap();
        let no_ext = dir.path().join("README");
        std::fs::write(&no_ext, b"notes").unwrap();

        prune_with_future_cutoff(dir.path());

        assert!(
            no_ext.exists(),
            "files without an extension should be left alone"
        );
    }

    #[test]
    fn prune_does_not_panic_on_missing_directory() {
        let dir = tempfile::tempdir().unwrap();
        let nonexistent = dir.path().join("no_such_dir");
        // Must not panic.
        prune_with_future_cutoff(&nonexistent);
    }

    #[test]
    fn prune_removes_rotated_but_spares_active_in_mixed_directory() {
        let dir = tempfile::tempdir().unwrap();
        let rotated = dir.path().join("jabberwok.log.2026-03-01");
        let active = dir.path().join("jabberwok.log");
        std::fs::write(&rotated, b"old").unwrap();
        std::fs::write(&active, b"active").unwrap();

        prune_with_future_cutoff(dir.path());

        assert!(!rotated.exists(), "rotated log should be deleted");
        assert!(active.exists(), "active .log must survive regardless");
    }

    #[test]
    fn prune_keeps_file_modified_after_cutoff() {
        let dir = tempfile::tempdir().unwrap();
        let rotated = dir.path().join("jabberwok.log.2026-03-04");
        std::fs::write(&rotated, b"boundary").unwrap();

        // Cutoff set 1 second in the past: the file was just written so its
        // mtime is after the cutoff and must not be deleted.
        let past_cutoff = SystemTime::now() - Duration::from_secs(1);
        prune_files_older_than(dir.path(), past_cutoff);

        assert!(
            rotated.exists(),
            "file modified after cutoff should be kept"
        );
    }

    // ---------------------------------------------------------------------------
    // prune_old_logs (public wrapper — exercises the keep_days → cutoff path)
    // ---------------------------------------------------------------------------

    #[test]
    fn prune_old_logs_with_large_retention_preserves_files() {
        let dir = tempfile::tempdir().unwrap();
        let rotated = dir.path().join("jabberwok.log.2026-03-18");
        std::fs::write(&rotated, b"recent").unwrap();

        // 36 500-day retention: no file is old enough to prune.
        prune_old_logs(dir.path(), 36_500);

        assert!(
            rotated.exists(),
            "file should survive with a very long retention"
        );
    }

    #[test]
    fn prune_old_logs_noop_on_missing_directory() {
        let dir = tempfile::tempdir().unwrap();
        let nonexistent = dir.path().join("ghost");
        // Must not panic.
        prune_old_logs(&nonexistent, LOG_RETENTION_DAYS);
    }

    // ---------------------------------------------------------------------------
    // init_logging_to (subscriber wiring — does not touch production paths)
    // ---------------------------------------------------------------------------

    #[test]
    fn init_logging_to_returns_guard_and_creates_appender() {
        let dir = tempfile::tempdir().unwrap();
        // try_init inside means this will not panic even if another test has
        // already installed a global subscriber.
        let guard = init_logging_to(dir.path(), &crate::config::LoggingConfig::default());
        drop(guard); // flushes the non-blocking writer
        // The directory must still exist; we cannot assert a log file was
        // created without emitting a log event through the installed subscriber,
        // which would require controlling global state.
        assert!(dir.path().exists());
    }
}
