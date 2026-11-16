//! File-based logging. Logs never go to the terminal, since the TUI owns it.
//!
//! SQL text must not be logged at info level (see requirement 23.3).

use std::fs::DirBuilder;
use std::io;
use std::os::unix::fs::DirBuilderExt;
use std::path::PathBuf;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::EnvFilter;

/// Returns the directory that holds log files.
///
/// Uses `$XDG_STATE_HOME/oxpecker`, falling back to `$HOME/.local/state/oxpecker`.
/// Returns `None` when neither variable is usable.
fn log_dir() -> Option<PathBuf> {
    let base = match std::env::var_os("XDG_STATE_HOME") {
        Some(p) if !p.is_empty() => PathBuf::from(p),
        _ => {
            let home = std::env::var_os("HOME").filter(|h| !h.is_empty())?;
            PathBuf::from(home).join(".local/state")
        }
    };
    Some(base.join("oxpecker"))
}

/// Creates the directory with owner-only permissions.
fn create_private_dir(path: &PathBuf) -> io::Result<()> {
    DirBuilder::new().recursive(true).mode(0o700).create(path)
}

/// Starts file logging. Keep the returned guard alive until exit so buffered
/// lines are flushed.
///
/// Logging is best effort: if the log directory cannot be created, the
/// application still starts and returns `None`.
pub fn init() -> Option<WorkerGuard> {
    let dir = log_dir()?;
    create_private_dir(&dir).ok()?;

    let appender = tracing_appender::rolling::daily(&dir, "oxpecker.log");
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let filter = EnvFilter::try_from_env("OXPECKER_LOG").unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::fmt()
        .with_writer(writer)
        .with_ansi(false)
        .with_env_filter(filter)
        .try_init()
        .ok()?;
    Some(guard)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn private_dir_is_owner_only() {
        let base = std::env::temp_dir().join(format!("oxpecker-test-{}", std::process::id()));
        let dir = base.join("a/b");
        create_private_dir(&dir).expect("create dir");
        let mode = std::fs::metadata(&dir)
            .expect("metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o077, 0, "group/other bits must be clear");
        std::fs::remove_dir_all(&base).expect("cleanup");
    }
}
