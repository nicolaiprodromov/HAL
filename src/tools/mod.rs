//! Agent tool implementations.
//!
//! Each tool implements rig's [`rig::tool::Tool`] trait and carries the
//! shared state it needs (memory, project paths, optional LLM client).

use std::sync::Arc;
use std::time::Instant;

use crate::control::PauseController;
use crate::events::{next_call_id, Agent, Event, EventSender};
use crate::llm::LlmClient;
use crate::memory::Memory;
use crate::project::Project;

pub mod files;
pub mod memory;
pub mod shell;
pub mod spawn_agent;
pub mod web_search;

/// Shared state handed to every tool instance. Cheap to clone (all Arc).
#[derive(Clone)]
pub struct ToolContext {
    pub project: Arc<Project>,
    pub memory: Arc<Memory>,
    pub agent: Agent,
    pub llm: Option<Arc<LlmClient>>,
    pub events: Option<EventSender>,
    pub pause: Option<Arc<PauseController>>,
}

impl ToolContext {
    pub fn new(project: Arc<Project>, memory: Arc<Memory>, agent: Agent) -> Self {
        Self {
            project,
            memory,
            agent,
            llm: None,
            events: None,
            pause: None,
        }
    }
    pub fn with_llm(mut self, llm: Arc<LlmClient>) -> Self {
        self.llm = Some(llm);
        self
    }
    pub fn with_events(mut self, events: EventSender) -> Self {
        self.events = Some(events);
        self
    }
    pub fn with_pause(mut self, pause: Arc<PauseController>) -> Self {
        self.pause = Some(pause);
        self
    }

    /// Announce a tool call on the bus and return a guard that emits
    /// the matching `ToolCallEnd` when `finish_ok` / `finish_err` is
    /// called. If no `events` sender is configured this is a no-op.
    ///
    /// `args` carries the *complete* argument payload (no truncation) —
    /// the UI is responsible for collapsing long values.
    pub fn begin_call(&self, tool: &str, args: String) -> ToolCallGuard {
        let call_id = next_call_id();
        if let Some(tx) = &self.events {
            let _ = tx.send(Event::ToolCallStart {
                agent: self.agent,
                call_id,
                tool: tool.to_string(),
                args_preview: args,
            });
        }
        ToolCallGuard {
            events: self.events.clone(),
            agent: self.agent,
            call_id,
            tool: tool.to_string(),
            started: Instant::now(),
            finished: false,
        }
    }
}

/// RAII-ish guard for one tool call. If dropped without an explicit
/// finish, it emits a synthetic failure so UI state stays consistent.
pub struct ToolCallGuard {
    events: Option<EventSender>,
    agent: Agent,
    call_id: u64,
    tool: String,
    started: Instant,
    finished: bool,
}

impl ToolCallGuard {
    pub fn finish_ok(mut self, output_preview: String) {
        self.emit(true, output_preview);
    }
    pub fn finish_err(mut self, error_preview: String) {
        self.emit(false, error_preview);
    }
    fn emit(&mut self, ok: bool, preview: String) {
        if self.finished {
            return;
        }
        self.finished = true;
        if let Some(tx) = &self.events {
            let _ = tx.send(Event::ToolCallEnd {
                agent: self.agent,
                call_id: self.call_id,
                tool: self.tool.clone(),
                ok,
                output_preview: preview,
                duration_ms: self.started.elapsed().as_millis() as u64,
            });
        }
    }
}

impl Drop for ToolCallGuard {
    fn drop(&mut self) {
        if !self.finished {
            self.emit(false, "tool call dropped without result".into());
        }
    }
}

/// Truncate a string to a char count with an ellipsis.
pub fn preview(s: &str, max_chars: usize) -> String {
    let count = s.chars().count();
    if count <= max_chars {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max_chars).collect();
    out.push('…');
    out
}

/// Canonicalize `candidate` and ensure it stays inside one of the allowed
/// roots. Used by every file-touching tool to block path traversal.
pub(crate) fn resolve_sandboxed(
    candidate: &std::path::Path,
    roots: &[std::path::PathBuf],
) -> anyhow::Result<std::path::PathBuf> {
    use anyhow::Context;
    let abs = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        roots
            .first()
            .context("no sandbox root configured")?
            .join(candidate)
    };

    let canon = if abs.exists() {
        std::fs::canonicalize(&abs).with_context(|| format!("canonicalize {}", abs.display()))?
    } else {
        let parent = abs
            .parent()
            .with_context(|| format!("path has no parent: {}", abs.display()))?;
        std::fs::create_dir_all(parent).ok();
        let parent_canon = std::fs::canonicalize(parent)
            .with_context(|| format!("canonicalize parent {}", parent.display()))?;
        let file_name = abs
            .file_name()
            .with_context(|| format!("path has no file name: {}", abs.display()))?;
        parent_canon.join(file_name)
    };

    let mut allowed = false;
    for root in roots {
        let Ok(root_canon) = std::fs::canonicalize(root) else {
            continue;
        };
        if canon.starts_with(&root_canon) || canon == root_canon {
            allowed = true;
            break;
        }
    }
    if !allowed {
        anyhow::bail!("path {} escapes the allowed sandbox roots", canon.display());
    }
    Ok(canon)
}

#[cfg(test)]
mod sandbox_tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn allows_paths_inside_root() {
        let dir = tempdir().unwrap();
        let sub = dir.path().join("sub");
        fs::create_dir_all(&sub).unwrap();
        let f = sub.join("ok.txt");
        fs::write(&f, b"hi").unwrap();
        let r = resolve_sandboxed(&f, &[dir.path().to_path_buf()]).unwrap();
        assert!(r.ends_with("ok.txt"));
    }

    #[test]
    fn rejects_paths_outside_root() {
        let dir = tempdir().unwrap();
        let outside = tempdir().unwrap();
        let f = outside.path().join("evil.txt");
        fs::write(&f, b"x").unwrap();
        let err = resolve_sandboxed(&f, &[dir.path().to_path_buf()]).unwrap_err();
        assert!(format!("{err}").contains("escapes"));
    }

    #[test]
    fn rejects_dotdot_traversal() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("root");
        fs::create_dir_all(&root).unwrap();
        let victim = dir.path().join("victim.txt");
        fs::write(&victim, b"secret").unwrap();
        let traverse = root.join("..").join("victim.txt");
        let err = resolve_sandboxed(&traverse, &[root]).unwrap_err();
        assert!(format!("{err}").contains("escapes"));
    }
}
