//! Centralised logging for HAL.
//!
//! A `hal` invocation writes two streams:
//!
//! 1. A rotating, structured file log at `~/.hal/logs/hal.log` (daily roll).
//!    Always on. Survives across runs so the user can grep for a crash
//!    long after the TUI has torn down.
//! 2. A stderr stream that is **only** active when explicitly requested
//!    via `HAL_LOG_STDERR=1` (or `-v` flag). Off by default because
//!    stderr would otherwise fight the ratatui dashboard for the screen.
//!
//! Filtering uses the standard `RUST_LOG` / `HAL_LOG` env-var grammar
//! (see `tracing_subscriber::EnvFilter`). Default is
//! `hal=debug,bollard=info,rig=info` which is chatty enough to see every
//! agent decision without drowning in reqwest / hyper internals.
//!
//! The intent: **when HAL crashes, the file log is the first thing to
//! look at**. It records every phase transition, every agent message,
//! every tool call (start + end), every container lifecycle event, plus
//! the full error chain on `anyhow::Error` bubble-ups.

use std::path::PathBuf;

use tracing::Level;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling;
use tracing_subscriber::fmt::time::ChronoUtc;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, EnvFilter};

/// Flush guard returned from [`init`]. Drop it at program exit so the
/// background writer thread gets a chance to flush any buffered lines.
pub struct LogGuard {
    _file: WorkerGuard,
    pub log_file: PathBuf,
}

/// Resolve the directory under which `hal.log` should live.
///
/// Priority:
/// 1. `HAL_LOG_DIR` env var.
/// 2. `$XDG_STATE_HOME/hal/logs` if set.
/// 3. `$HOME/.hal/logs`.
/// 4. `/tmp/hal-logs` as a last resort.
pub fn default_log_dir() -> PathBuf {
    if let Ok(v) = std::env::var("HAL_LOG_DIR") {
        if !v.is_empty() {
            return PathBuf::from(v);
        }
    }
    if let Ok(v) = std::env::var("XDG_STATE_HOME") {
        if !v.is_empty() {
            return PathBuf::from(v).join("hal").join("logs");
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        if !home.is_empty() {
            return PathBuf::from(home).join(".hal").join("logs");
        }
    }
    PathBuf::from("/tmp/hal-logs")
}

/// Initialise the global tracing subscriber. Call exactly once, as early
/// as possible in `main`. Returns a guard whose drop flushes the log.
///
/// `stderr` forces on the stderr layer regardless of env settings — used
/// by `-d/--direct` mode only for fatal errors. Normal TUI runs set
/// `stderr=false` so the dashboard isn't polluted.
pub fn init(stderr: bool) -> anyhow::Result<LogGuard> {
    let dir = default_log_dir();
    std::fs::create_dir_all(&dir)
        .map_err(|e| anyhow::anyhow!("creating log dir {}: {e}", dir.display()))?;

    let file_appender = rolling::daily(&dir, "hal.log");
    let (file_writer, file_guard) = tracing_appender::non_blocking(file_appender);

    let filter = std::env::var("HAL_LOG")
        .or_else(|_| std::env::var("RUST_LOG"))
        .unwrap_or_else(|_| "hal=debug,bollard=info,rig=info".to_string());

    let file_layer = fmt::layer()
        .with_writer(file_writer)
        .with_ansi(false)
        .with_target(true)
        .with_thread_ids(false)
        .with_thread_names(false)
        .with_level(true)
        .with_timer(ChronoUtc::rfc_3339())
        .with_file(true)
        .with_line_number(true);

    let env_stderr = std::env::var("HAL_LOG_STDERR")
        .map(|v| matches!(v.trim(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false);

    let stderr_enabled = stderr || env_stderr;

    let registry = tracing_subscriber::registry()
        .with(EnvFilter::try_new(&filter).unwrap_or_else(|_| EnvFilter::new("hal=info")))
        .with(file_layer);

    if stderr_enabled {
        let stderr_layer = fmt::layer()
            .with_writer(std::io::stderr)
            .with_ansi(true)
            .with_target(false)
            .with_level(true)
            .with_timer(ChronoUtc::rfc_3339());
        registry.with(stderr_layer).try_init().ok();
    } else {
        registry.try_init().ok();
    }

    let log_file = dir.join("hal.log");
    tracing::info!(
        target: "hal::boot",
        version = env!("CARGO_PKG_VERSION"),
        log_file = %log_file.display(),
        filter = %filter,
        "hal logging initialised"
    );

    Ok(LogGuard {
        _file: file_guard,
        log_file,
    })
}

/// Minimal helper for code paths that want to log a panic-adjacent fact
/// at a high level without taking a dep on `tracing` macros directly.
pub fn emit(level: Level, target: &'static str, msg: &str) {
    match level {
        Level::ERROR => tracing::error!(target: "hal::emit", %target, "{msg}"),
        Level::WARN => tracing::warn!(target: "hal::emit", %target, "{msg}"),
        Level::INFO => tracing::info!(target: "hal::emit", %target, "{msg}"),
        Level::DEBUG => tracing::debug!(target: "hal::emit", %target, "{msg}"),
        Level::TRACE => tracing::trace!(target: "hal::emit", %target, "{msg}"),
    }
}
