//! HAL — Heuristic Agentic Language runtime library surface.
//!
//! This crate is both a binary (`hal`) and a library used by integration
//! tests. The module layout mirrors the milestones in `HAL/init.md`.

pub mod config;
pub mod control;
pub mod events;
pub mod logging;
pub mod project;

pub mod http_retry;
pub mod llm;
pub mod memory;

pub mod tools;

pub mod agents;
pub mod pipeline;

pub mod container;

pub mod tui;

pub mod commands;

/// Embedded identity documents (loaded via `include_str!`) so the binary
/// ships without needing the `HAL/*.md` files at runtime.
pub mod identity {
    pub const INTERPRETER: &str = include_str!("../HAL/interpreter.md");
    pub const ARCHITECT: &str = include_str!("../HAL/architect.md");
    pub const CONDUCTOR: &str = include_str!("../HAL/conductor.md");
    pub const REFLECTOR: &str = include_str!("../HAL/reflector.md");
    pub const TOOLS: &str = include_str!("../HAL/tools.md");
    pub const FLOW: &str = include_str!("../HAL/flow.md");
}
