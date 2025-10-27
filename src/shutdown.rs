//! Process-wide signal handling for `hal run` / `hal build`.
//!
//! Whatever way the user (or the OS) ends HAL, the container must be
//! stopped and removed and the terminal restored:
//!
//! - **SIGINT** (Ctrl-C when the terminal is cooked, `kill -INT`),
//!   **SIGTERM**, **SIGHUP** (terminal closed) are all caught. Without a
//!   handler the default action kills `hal` on the spot and the container
//!   keeps running with its published port.
//! - The first signal asks the container to stop (SIGINT to its process
//!   group, SIGKILL after [`container::INTERRUPT_GRACE`]). A second
//!   signal, or the safety deadline, force-removes it and exits.
//! - In TUI mode the first signal asks the TUI to close; the normal exit
//!   path in `commands::run` then stops the container.
//!
//! SIGKILL cannot be caught; [`container::reap_stale`] covers it on the
//! next start.

use std::io::Write;
use std::time::Duration;

use tokio::sync::broadcast::error::RecvError;
use tokio::sync::broadcast::Receiver;

use crate::container;
use crate::events::{Event, Phase};
use crate::tui;

/// Print to stderr without ever panicking. `eprintln!` panics when the
/// terminal is gone (SIGHUP), which would abort the cleanup.
pub fn say(msg: &str) {
    let _ = writeln!(std::io::stderr(), "{msg}");
}

/// Remove containers left behind by `hal` processes that died hard.
/// Best effort and bounded: a missing or slow Docker daemon never blocks.
pub async fn reap_stale_containers() {
    let Ok(docker) = container::connect() else {
        return;
    };
    let reaped = tokio::time::timeout(Duration::from_secs(5), container::reap_stale(&docker))
        .await
        .unwrap_or_default();
    if !reaped.is_empty() {
        say(&format!(
            "hal: removed {} stale container(s) from an earlier run: {}",
            reaped.len(),
            reaped.join(", ")
        ));
    }
}

#[cfg(unix)]
struct Signals {
    int: tokio::signal::unix::Signal,
    term: tokio::signal::unix::Signal,
    hup: tokio::signal::unix::Signal,
}

#[cfg(unix)]
impl Signals {
    fn new() -> std::io::Result<Self> {
        use tokio::signal::unix::{signal, SignalKind};
        Ok(Self {
            int: signal(SignalKind::interrupt())?,
            term: signal(SignalKind::terminate())?,
            hup: signal(SignalKind::hangup())?,
        })
    }

    /// Wait for the next signal; returns its conventional exit code.
    async fn next(&mut self) -> i32 {
        tokio::select! {
            _ = self.int.recv() => 130,
            _ = self.term.recv() => 143,
            _ = self.hup.recv() => 129,
        }
    }
}

#[cfg(not(unix))]
struct Signals;

#[cfg(not(unix))]
impl Signals {
    fn new() -> std::io::Result<Self> {
        Ok(Self)
    }

    async fn next(&mut self) -> i32 {
        let _ = tokio::signal::ctrl_c().await;
        130
    }
}

/// Install the handler. `tui_mode` selects who closes the terminal UI.
/// Must be called from inside the tokio runtime, before any container
/// starts. `events` is used to know whether the program itself is running.
pub fn spawn_signal_handler(mut events: Receiver<Event>, tui_mode: bool) {
    let Ok(mut signals) = Signals::new() else {
        return;
    };
    tokio::spawn(async move {
        let mut running = false;
        let mut bus_open = true;
        let first = loop {
            tokio::select! {
                evt = events.recv(), if bus_open => match evt {
                    Ok(Event::Phase(p)) => running = matches!(p, Phase::Running),
                    Ok(_) | Err(RecvError::Lagged(_)) => {}
                    Err(RecvError::Closed) => bus_open = false,
                },
                code = signals.next() => break code,
            }
        };

        // Stage 1: stop politely if there is something to stop politely.
        // A signal that arrives while a stop is already under way (the
        // second Ctrl-C) skips straight to the forced exit.
        let graceful = if container::interrupted() {
            false
        } else if tui_mode {
            // The TUI closes; `commands::run` then interrupts the
            // container and waits for it.
            tui::app::request_quit();
            true
        } else if container::active_runs() > 0 && (running || first != 130) {
            say("\nhal: interrupt; stopping the container (Ctrl-C again to force quit)");
            container::interrupt_runs();
            true
        } else {
            false
        };

        let mut code = first;
        if graceful {
            // Safety deadline: grace period + SIGKILL + removal slack.
            let deadline = container::INTERRUPT_GRACE + Duration::from_secs(10);
            tokio::select! {
                c = signals.next() => code = c,
                _ = tokio::time::sleep(deadline) => {}
            }
        }

        // Stage 2: nothing graceful left. Make sure nothing is stranded.
        tui::app::restore_terminal();
        container::shutdown_cleanup().await;
        std::process::exit(code);
    });
}
