//! `spawn_agent` — Architect-only. Launches a stateless worker agent
//! with a focused prompt and optional JSON schema for the return value.
//! Workers have no memory access and cannot spawn further agents.
//!
//! Concurrency is bounded by a global semaphore (cap = 4, per plan §3.2).

use std::sync::{Arc, LazyLock};

use rig::client::CompletionClient;
use rig::completion::Prompt;
use rig::completion::ToolDefinition;
use rig::tool::Tool;
use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;

use super::ToolContext;
use crate::events::Agent;

static WORKER_SEM: LazyLock<Arc<Semaphore>> = LazyLock::new(|| Arc::new(Semaphore::new(4)));

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct SpawnAgentError(String);

#[derive(Debug, Deserialize)]
pub struct SpawnArgs {
    /// Focused task prompt.
    pub prompt: String,
    /// Optional JSON Schema the worker's response must conform to.
    #[serde(default)]
    pub schema: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub struct SpawnOut {
    pub output: String,
}

#[derive(Clone)]
pub struct SpawnAgent {
    pub ctx: ToolContext,
}

impl Tool for SpawnAgent {
    const NAME: &'static str = "spawn_agent";
    type Error = SpawnAgentError;
    type Args = SpawnArgs;
    type Output = SpawnOut;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.into(),
            description: "Launch a stateless worker agent to perform a focused task. Architect-only. Workers have no memory and cannot spawn further agents.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "prompt": {"type": "string"},
                    "schema": {"type": "object", "description": "Optional JSON Schema for the return value."}
                },
                "required": ["prompt"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let guard = self
            .ctx
            .begin_call(Self::NAME, format!("prompt={:?}", args.prompt));
        if !matches!(self.ctx.agent, Agent::Architect) {
            let msg = "spawn_agent is only available to the Architect".to_string();
            guard.finish_err(msg.clone());
            return Err(SpawnAgentError(msg));
        }
        let llm = match self.ctx.llm.clone() {
            Some(l) => l,
            None => {
                let msg = "spawn_agent requires an LLM client".to_string();
                guard.finish_err(msg.clone());
                return Err(SpawnAgentError(msg));
            }
        };

        let _permit = match WORKER_SEM.clone().acquire_owned().await {
            Ok(p) => p,
            Err(e) => {
                let msg = format!("worker semaphore closed: {e}");
                guard.finish_err(msg.clone());
                return Err(SpawnAgentError(msg));
            }
        };

        let preamble = worker_preamble(args.schema.as_ref());
        let agent = llm.client.agent(&llm.model).preamble(&preamble).build();

        match agent.prompt(args.prompt.as_str()).await {
            Ok(output) => {
                guard.finish_ok(format!("worker replied ({} chars)", output.chars().count()));
                Ok(SpawnOut { output })
            }
            Err(e) => {
                let msg = format!("worker prompt failed: {e}");
                guard.finish_err(msg.clone());
                Err(SpawnAgentError(msg))
            }
        }
    }
}

fn worker_preamble(schema: Option<&serde_json::Value>) -> String {
    let mut s = String::from(
        "You are a stateless worker agent spawned by the HAL Architect. \
         Focus on the single task given to you. Respond concisely. \
         You have no memory and cannot spawn further agents.",
    );
    if let Some(sc) = schema {
        s.push_str("\n\nYour response MUST be a JSON document conforming to this schema:\n");
        s.push_str(&serde_json::to_string_pretty(sc).unwrap_or_else(|_| sc.to_string()));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::Memory;
    use crate::project::Project;
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
    async fn refuses_non_architect_callers() {
        let (_d, ctx) = ctx(Agent::Interpreter).await;
        let err = SpawnAgent { ctx }
            .call(SpawnArgs {
                prompt: "hi".into(),
                schema: None,
            })
            .await
            .unwrap_err();
        assert!(format!("{err}").contains("Architect"));
    }

    #[tokio::test]
    async fn requires_an_llm_client() {
        let (_d, ctx) = ctx(Agent::Architect).await;
        let err = SpawnAgent { ctx }
            .call(SpawnArgs {
                prompt: "hi".into(),
                schema: None,
            })
            .await
            .unwrap_err();
        assert!(format!("{err}").contains("LLM"));
    }
}
