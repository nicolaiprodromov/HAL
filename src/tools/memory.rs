//! `memory_search` / `memory_write` tools.

use rig::completion::ToolDefinition;
use rig::tool::Tool;
use serde::{Deserialize, Serialize};

use super::ToolContext;
use crate::events::Agent;

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct MemoryToolError(String);

impl From<anyhow::Error> for MemoryToolError {
    fn from(e: anyhow::Error) -> Self {
        Self(format!("{e:#}"))
    }
}

#[derive(Debug, Deserialize)]
pub struct SearchArgs {
    pub query: String,
    #[serde(default = "default_k")]
    pub k: usize,
    /// Optional namespace override. Values: `"interpreter"`, `"architect"`,
    /// `"conductor"`, or `"self"` (default — the calling agent).
    /// Use this from Interpreter/Architect to look up past runtime
    /// observations written by the Conductor/Reflector.
    #[serde(default)]
    pub namespace: Option<String>,
}
fn default_k() -> usize {
    5
}

fn parse_namespace(s: &str, caller: Agent) -> Result<Agent, MemoryToolError> {
    match s.trim().to_ascii_lowercase().as_str() {
        "" | "self" | "me" => Ok(caller),
        "interpreter" => Ok(Agent::Interpreter),
        "architect" => Ok(Agent::Architect),
        "conductor" | "reflector" => Ok(Agent::Conductor),
        other => Err(MemoryToolError(format!(
            "unknown namespace {other:?}; expected one of: self, interpreter, architect, conductor"
        ))),
    }
}

#[derive(Debug, Serialize)]
pub struct SearchHit {
    pub id: i64,
    pub content: String,
    pub created_at: i64,
}

#[derive(Clone)]
pub struct MemorySearch {
    pub ctx: ToolContext,
}

impl Tool for MemorySearch {
    const NAME: &'static str = "memory_search";
    type Error = MemoryToolError;
    type Args = SearchArgs;
    type Output = Vec<SearchHit>;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.into(),
            description:
                "Search memory notes. Defaults to the calling agent's own namespace. Pass namespace=\"conductor\" to recall runtime observations written by the Reflector after past container runs — Interpreter/Architect should ALWAYS do this before making structural decisions."
                    .into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "query":     {"type": "string"},
                    "k":         {"type": "integer", "minimum": 1, "default": 5},
                    "namespace": {"type": "string", "enum": ["self","interpreter","architect","conductor"], "default": "self"}
                },
                "required": ["query"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let target = match args.namespace.as_deref() {
            None => self.ctx.agent,
            Some(s) => parse_namespace(s, self.ctx.agent)?,
        };
        let guard = self.ctx.begin_call(
            Self::NAME,
            format!(
                "query={:?} k={} ns={}",
                args.query,
                args.k,
                agent_label(target)
            ),
        );
        match self.ctx.memory.search(target, &args.query, args.k).await {
            Ok(hits) => {
                let out: Vec<SearchHit> = hits
                    .into_iter()
                    .map(|n| SearchHit {
                        id: n.id,
                        content: n.content,
                        created_at: n.created_at,
                    })
                    .collect();
                guard.finish_ok(format!("{} hit(s) in {}", out.len(), agent_label(target)));
                Ok(out)
            }
            Err(e) => {
                let msg = format!("{e:#}");
                guard.finish_err(msg.clone());
                Err(MemoryToolError(msg))
            }
        }
    }
}

fn agent_label(a: Agent) -> &'static str {
    match a {
        Agent::Interpreter => "interpreter",
        Agent::Architect => "architect",
        Agent::Conductor => "conductor",
    }
}

#[derive(Debug, Deserialize)]
pub struct WriteArgs {
    pub content: String,
}

#[derive(Debug, Serialize)]
pub struct WriteOut {
    pub id: i64,
}

#[derive(Clone)]
pub struct MemoryWrite {
    pub ctx: ToolContext,
}

impl Tool for MemoryWrite {
    const NAME: &'static str = "memory_write";
    type Error = MemoryToolError;
    type Args = WriteArgs;
    type Output = WriteOut;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.into(),
            description: "Append a note to the calling agent's memory namespace.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {"content": {"type": "string"}},
                "required": ["content"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let guard = self
            .ctx
            .begin_call(Self::NAME, format!("content={:?}", args.content));
        match self.ctx.memory.write(self.ctx.agent, &args.content).await {
            Ok(id) => {
                guard.finish_ok(format!("note #{id}"));
                Ok(WriteOut { id })
            }
            Err(e) => {
                let msg = format!("{e:#}");
                guard.finish_err(msg.clone());
                Err(MemoryToolError(msg))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::Agent;
    use crate::memory::Memory;
    use crate::project::Project;
    use std::sync::Arc;
    use tempfile::tempdir;

    async fn ctx(agent: Agent) -> (tempfile::TempDir, ToolContext) {
        let d = tempdir().unwrap();
        let src = d.path().join("d.hal");
        std::fs::write(&src, b"x").unwrap();
        let p = Project::open(&src).unwrap();
        p.ensure_layout().unwrap();
        let m = Memory::in_memory().await.unwrap();
        (d, ToolContext::new(Arc::new(p), Arc::new(m), agent))
    }

    #[tokio::test]
    async fn write_then_search() {
        let (_d, ctx) = ctx(Agent::Architect).await;
        let w = MemoryWrite { ctx: ctx.clone() };
        w.call(WriteArgs {
            content: "prefer bash for print-and-sleep tasks".into(),
        })
        .await
        .unwrap();

        let s = MemorySearch { ctx };
        let hits = s
            .call(SearchArgs {
                query: "bash".into(),
                k: 5,
                namespace: None,
            })
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].content.contains("bash"));
    }

    #[tokio::test]
    async fn agents_dont_see_each_others_notes() {
        let (_d, arch_ctx) = ctx(Agent::Architect).await;
        let mem = arch_ctx.memory.clone();
        let interp_ctx = ToolContext::new(arch_ctx.project.clone(), mem, Agent::Interpreter);

        MemoryWrite { ctx: arch_ctx }
            .call(WriteArgs {
                content: "architect-only note".into(),
            })
            .await
            .unwrap();

        let hits = MemorySearch { ctx: interp_ctx }
            .call(SearchArgs {
                query: "architect".into(),
                k: 5,
                namespace: None,
            })
            .await
            .unwrap();
        assert!(hits.is_empty(), "interpreter must not see architect notes");
    }
}
