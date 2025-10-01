//! Pipeline orchestrator — sequentially runs Interpreter → Architect
//! (→ Conductor in M5), emitting events on a broadcast channel so the
//! TUI (or a plain log sink) can render progress.

use std::sync::Arc;

use anyhow::Result;

use crate::agents;
use crate::control::PauseController;
use crate::events::{self, Event, EventSender, Phase};
use crate::llm::LlmClient;
use crate::memory::Memory;
use crate::project::{BuildLock, Project};

/// Outcome of the build phase: produced `logic.md` + `artifacts/` +
/// `runtime.yaml`, and refreshed `build.lock`.
pub struct BuildOutcome {
    pub logic_md: String,
}

/// Run Interpreter + Architect. Caller is expected to invoke `run_conductor`
/// separately for `hal run`.
///
/// `runtime_args` are the tokens the user will pass to `hal run` after
/// the source path. They are surfaced to both agents so the produced
/// `runtime.yaml` and artifact bundle actually plan for them, instead
/// of the Architect guessing that the program takes no arguments.
pub async fn run_build(
    project: Arc<Project>,
    memory: Arc<Memory>,
    llm: Arc<LlmClient>,
    events_tx: &EventSender,
    pause: Option<Arc<PauseController>>,
    runtime_args: &[String],
) -> Result<BuildOutcome> {
    if let Some(p) = pause.as_ref() {
        p.await_if_paused().await;
    }
    let _ = events_tx.send(Event::Phase(Phase::Interpreting));
    let logic_md = agents::interpreter::run(
        project.clone(),
        memory.clone(),
        llm.clone(),
        events_tx,
        pause.clone(),
        runtime_args,
    )
    .await?;

    if let Some(p) = pause.as_ref() {
        p.await_if_paused().await;
    }
    let _ = events_tx.send(Event::Phase(Phase::Building));
    agents::architect::run(
        project.clone(),
        memory.clone(),
        llm.clone(),
        events_tx,
        pause.clone(),
        runtime_args,
    )
    .await?;

    // Freeze build.lock. Parse runtime.yaml to extract image + entrypoint.
    let (image, entrypoint) = read_runtime_brief(&project)?;
    let lock = BuildLock {
        source_hash: project.source_hash()?,
        built_at: chrono::Utc::now(),
        image,
        entrypoint,
    };
    project.write_build_lock(&lock)?;

    Ok(BuildOutcome { logic_md })
}

/// Convenience helper for callers that don't need the event bus.
pub fn make_channel() -> (EventSender, tokio::sync::broadcast::Receiver<Event>) {
    events::channel(256)
}

#[derive(Debug, serde::Deserialize)]
struct RuntimeYaml {
    #[serde(default = "default_image")]
    image: String,
    entrypoint: RuntimeEntrypoint,
}
#[derive(Debug, serde::Deserialize)]
struct RuntimeEntrypoint {
    command: String,
    #[serde(default)]
    #[allow(dead_code)]
    cwd: Option<String>,
}
fn default_image() -> String {
    crate::config::DEFAULT_IMAGE.into()
}

fn read_runtime_brief(project: &Project) -> Result<(String, String)> {
    let s = std::fs::read_to_string(project.runtime_yaml())?;
    let y: RuntimeYaml = serde_yaml::from_str(&s)?;
    Ok((y.image, y.entrypoint.command))
}
