//! Ratatui-based TUI for `hal build` / `hal run`. Three stacked panels:
//! phase banner, agent transcript, conductor output (stdout + stderr).
//!
//! The TUI is purely a consumer of [`crate::events::Event`]s on the
//! broadcast bus; it does not drive the pipeline. The `-d/--direct`
//! flag on the CLI bypasses this module and routes events to stdout
//! line-per-line instead.

pub mod app;
pub mod theme;

pub use app::run_tui;
