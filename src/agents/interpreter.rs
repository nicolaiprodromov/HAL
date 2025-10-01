//! Interpreter agent.
//!
//! Given the raw `.hal` source, produce a `logic.md` in the project's
//! `.hal/<id>/` root. The agent writes the file itself via `files_write`
//! so memory + web_search context can influence the final content.

use std::sync::Arc;

use anyhow::{Context, Result};
use rig::client::CompletionClient;
use rig::streaming::StreamingPrompt;

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
    web_search::WebSearch,
    ToolContext,
};

/// Maximum number of tool-call turns the Interpreter may take.
pub const MAX_TURNS: usize = 12;

pub async fn run(
    project: Arc<Project>,
    memory: Arc<Memory>,
    llm: Arc<LlmClient>,
    events: &EventSender,
    pause: Option<Arc<PauseController>>,
    runtime_args: &[String],
) -> Result<String> {
    let _ = events.send(Event::AgentLog {
        agent: AgentKind::Interpreter,
        message: "reading source…".into(),
    });
    let source_bytes = project.read_source()?;
    let source = String::from_utf8_lossy(&source_bytes).to_string();

    let mut ctx = ToolContext::new(project.clone(), memory.clone(), AgentKind::Interpreter)
        .with_events(events.clone());
    if let Some(p) = pause.clone() {
        ctx = ctx.with_pause(p);
    }

    let preamble = build_preamble(&project, &memory, runtime_args).await;
    let _ = events.send(Event::AgentLog {
        agent: AgentKind::Interpreter,
        message: format!("--> {}…", llm.model),
    });

    let agent = llm
        .client
        .agent(&llm.model)
        .preamble(&preamble)
        .tool(FilesRead { ctx: ctx.clone() })
        .tool(FilesWrite { ctx: ctx.clone() })
        .tool(mem_tools::MemorySearch { ctx: ctx.clone() })
        .tool(mem_tools::MemoryWrite { ctx: ctx.clone() })
        .tool(WebSearch::new().with_ctx(ctx.clone()))
        .build();

    let mut user_msg = format!(
        "Here is the source of `{}`.\n\n\
         Write the Logic Description to `{}` using the `files_write` tool.\n\
         The file must start with a YAML front-matter block per your instructions.\n\
         When done, reply with a single line: `LOGIC_WRITTEN`.\n\n\
         ---SOURCE---\n{}\n---END SOURCE---",
        project.source.display(),
        project.logic_md().display(),
        source
    );

    // Steering loop: if the user interrupts mid-turn, we receive the
    // queued messages and re-issue a fresh prompt that incorporates
    // them, up to a small retry cap. If the model ends its turn without
    // writing logic.md, ask once more.
    let logic_path = project.logic_md();
    let original_msg = user_msg.clone();
    let mut response = String::new();
    for attempt in 0..2 {
        for _ in 0..6 {
            let outcome = stream_helper::drive(
                agent.stream_prompt(user_msg.clone()).multi_turn(MAX_TURNS),
                AgentKind::Interpreter,
                events,
                pause.as_ref(),
            )
            .await
            .context("interpreter prompt failed")?;
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
        if logic_path.exists() || attempt == 1 {
            break;
        }
        let _ = events.send(Event::AgentLog {
            agent: AgentKind::Interpreter,
            message: "turn ended without logic.md; asking again".into(),
        });
        user_msg = build_missing_output_followup(&original_msg, &logic_path);
    }

    let _ = events.send(Event::AgentResponse {
        agent: AgentKind::Interpreter,
        text: response.clone(),
    });
    let _ = events.send(Event::AgentLog {
        agent: AgentKind::Interpreter,
        message: format!("done: {}", response.lines().next().unwrap_or("")),
    });

    // Ensure the file actually exists.
    if !logic_path.exists() {
        anyhow::bail!(
            "interpreter finished without writing {}",
            logic_path.display()
        );
    }
    let logic = std::fs::read_to_string(&logic_path)?;
    Ok(logic)
}

/// Compose a follow-up user message after the user pushed steering. The
/// model sees its own partial output plus every steer verbatim.
pub(crate) fn build_steering_followup(original: &str, partial: &str, steers: &[String]) -> String {
    let mut s = String::new();
    s.push_str(
        "The user paused the trajectory and injected steering. Incorporate it and continue.\n\n",
    );
    for (i, m) in steers.iter().enumerate() {
        s.push_str(&format!("## Steering message {}\n{}\n\n", i + 1, m));
    }
    if !partial.trim().is_empty() {
        s.push_str("## Your partial output so far (before interruption)\n```\n");
        s.push_str(partial);
        s.push_str("\n```\n\n");
    }
    s.push_str("## Original task (for reference)\n");
    s.push_str(original);
    s
}

/// Re-prompt for a turn that ended without writing its required file.
/// Prompts carry no history, so the original task is repeated.
pub(crate) fn build_missing_output_followup(original: &str, path: &std::path::Path) -> String {
    format!(
        "Your previous turn ended without writing `{}`, and that file does not exist yet. \
         Call `files_write` for it now; the task is not done until it exists.\n\n\
         ## Original task\n{}",
        path.display(),
        original
    )
}

async fn build_preamble(
    project: &Project,
    memory: &Arc<Memory>,
    runtime_args: &[String],
) -> String {
    let digest = super::context::memory_digest(memory, 5).await;
    let args_block = super::context::runtime_args_block(runtime_args);
    format!(
        "{identity}\n\n\
         {env}\n\
         ## Current project\n\
         - source: `{src}`\n\
         - project root: `{root}`\n\
         - write `logic.md` to: `{logic}`\n\n\
         {args}{digest}",
        identity = identity::INTERPRETER,
        env = super::context::env_block(),
        src = project.source.display(),
        root = project.root.display(),
        logic = project.logic_md().display(),
        args = args_block,
        digest = digest,
    )
}
