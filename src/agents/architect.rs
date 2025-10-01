//! Architect agent.
//!
//! Given a `logic.md`, produce the `artifacts/` directory contents and a
//! `runtime.yaml` brief for the Conductor.

use std::sync::Arc;

use anyhow::{Context, Result};
use rig::client::CompletionClient;
use rig::streaming::StreamingPrompt;

use crate::agents::interpreter::{build_missing_output_followup, build_steering_followup};
use crate::agents::streaming as stream_helper;
use crate::agents::streaming::DriveOutcome;
use crate::control::PauseController;
use crate::events::{Agent as AgentKind, Event, EventSender};
use crate::identity;
use crate::llm::LlmClient;
use crate::memory::Memory;
use crate::project::Project;
use crate::tools::{
    files::{FilesRead, FilesWrite},
    memory as mem_tools,
    shell::RunCmd,
    spawn_agent::SpawnAgent,
    web_search::WebSearch,
    ToolContext,
};

pub const MAX_TURNS: usize = 24;

pub async fn run(
    project: Arc<Project>,
    memory: Arc<Memory>,
    llm: Arc<LlmClient>,
    events: &EventSender,
    pause: Option<Arc<PauseController>>,
    runtime_args: &[String],
) -> Result<()> {
    let logic = std::fs::read_to_string(project.logic_md())
        .context("reading logic.md before architect run")?;

    let _ = events.send(Event::AgentLog {
        agent: AgentKind::Architect,
        message: format!("--> {}…", llm.model),
    });

    let mut ctx = ToolContext::new(project.clone(), memory.clone(), AgentKind::Architect)
        .with_llm(llm.clone())
        .with_events(events.clone());
    if let Some(p) = pause.clone() {
        ctx = ctx.with_pause(p);
    }

    let digest = super::context::memory_digest(&memory, 5).await;
    let args_block = super::context::runtime_args_block(runtime_args);
    let preamble = format!(
        "{identity}\n\n\
         {env}\n\
         ## Current project\n\
         - project root: `{root}`\n\
         - artifacts directory: `{art}`\n\
         - runtime.yaml path: `{rt}`\n\n\
         {args}{digest}",
        identity = identity::ARCHITECT,
        env = super::context::env_block(),
        root = project.root.display(),
        art = project.artifacts_dir().display(),
        rt = project.runtime_yaml().display(),
        args = args_block,
        digest = digest,
    );

    let agent = llm
        .client
        .agent(&llm.model)
        .preamble(&preamble)
        .tool(FilesRead { ctx: ctx.clone() })
        .tool(FilesWrite { ctx: ctx.clone() })
        .tool(mem_tools::MemorySearch { ctx: ctx.clone() })
        .tool(mem_tools::MemoryWrite { ctx: ctx.clone() })
        .tool(WebSearch::new().with_ctx(ctx.clone()))
        .tool(SpawnAgent { ctx: ctx.clone() })
        .tool(RunCmd { ctx: ctx.clone() })
        .build();

    let mut user_msg = format!(
        "Here is the Logic Description. Build the artifact bundle for it.\n\n\
         Use `files_write` to populate `{}` and to write `{}`.\n\
         When done, reply with a single line: `BUILD_COMPLETE`.\n\n\
         ---LOGIC.MD---\n{}\n---END LOGIC.MD---",
        project.artifacts_dir().display(),
        project.runtime_yaml().display(),
        logic,
    );

    // As in the Interpreter: steering re-prompts, plus one more ask if
    // the turn ends without runtime.yaml.
    let runtime_path = project.runtime_yaml();
    let original_msg = user_msg.clone();
    let mut response = String::new();
    for attempt in 0..2 {
        for _ in 0..6 {
            let outcome = stream_helper::drive(
                agent.stream_prompt(user_msg.clone()).multi_turn(MAX_TURNS),
                AgentKind::Architect,
                events,
                pause.as_ref(),
            )
            .await
            .context("architect prompt failed")?;
            match outcome {
                DriveOutcome::Finished { text } => {
                    response = text;
                    break;
                }
                DriveOutcome::Steered { text, steers } => {
                    user_msg = build_steering_followup(&user_msg, &text, &steers);
                }
            }
        }
        if runtime_path.exists() || attempt == 1 {
            break;
        }
        let _ = events.send(Event::AgentLog {
            agent: AgentKind::Architect,
            message: "turn ended without runtime.yaml; asking again".into(),
        });
        user_msg = build_missing_output_followup(&original_msg, &runtime_path);
    }

    let _ = events.send(Event::AgentResponse {
        agent: AgentKind::Architect,
        text: response.clone(),
    });
    let _ = events.send(Event::AgentLog {
        agent: AgentKind::Architect,
        message: format!("done: {}", response.lines().next().unwrap_or("")),
    });

    if !runtime_path.exists() {
        anyhow::bail!(
            "architect finished without writing {}",
            runtime_path.display()
        );
    }

    Ok(())
}
