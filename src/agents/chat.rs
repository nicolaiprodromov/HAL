//! Post-pipeline interactive chat with one of the three agents.
//!
//! After the pipeline (Interpret → Build → Run → Reflect) has finished,
//! the TUI keeps running and lets the user converse with any agent to
//! inspect decisions, read files, search memory, or ask questions. This
//! module provides [`turn`] — a single chat round for a specified
//! agent.
//!
//! Each turn reuses the same preamble shape the agent would see during
//! its normal pipeline role (so it carries its identity, host env, and
//! memory digest), plus the running chat history, and ends with the
//! new user message. All tools available to that agent during the
//! pipeline are available here too, so the agent can run commands and
//! read/write files while chatting.

use std::sync::Arc;

use anyhow::{Context, Result};
use rig::client::CompletionClient;
use rig::streaming::StreamingPrompt;

use crate::agents::streaming::DriveOutcome;
use crate::agents::{context as agents_ctx, streaming as stream_helper};
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
    web_search::WebSearch,
    ToolContext,
};

/// One turn of a prior conversation. The full transcript is re-included
/// in every prompt so the agent has the context of earlier exchanges.
#[derive(Debug, Clone)]
pub struct ChatTurn {
    pub agent: AgentKind,
    pub user: String,
    pub assistant: String,
}

const MAX_TURNS: usize = 12;

/// Run a single chat turn for `agent` against `user_msg`. Emits the
/// normal streaming events (thinking, tokens, tool calls, final
/// response) on the event bus, and returns the assistant's text so the
/// caller can append it to history.
#[allow(clippy::too_many_arguments)]
pub async fn turn(
    agent_kind: AgentKind,
    project: Arc<Project>,
    memory: Arc<Memory>,
    llm: Arc<LlmClient>,
    events: &EventSender,
    pause: Option<Arc<PauseController>>,
    user_msg: String,
    history: &[ChatTurn],
) -> Result<String> {
    let _ = events.send(Event::AgentLog {
        agent: agent_kind,
        message: format!("chat → {}…", llm.model),
    });

    let mut ctx = ToolContext::new(project.clone(), memory.clone(), agent_kind)
        .with_llm(llm.clone())
        .with_events(events.clone());
    if let Some(p) = pause.clone() {
        ctx = ctx.with_pause(p);
    }

    let preamble = build_preamble(agent_kind, &project, &memory).await;
    let rig_agent = llm
        .client
        .agent(&llm.model)
        .preamble(&preamble)
        .tool(FilesRead { ctx: ctx.clone() })
        .tool(FilesWrite { ctx: ctx.clone() })
        .tool(mem_tools::MemorySearch { ctx: ctx.clone() })
        .tool(mem_tools::MemoryWrite { ctx: ctx.clone() })
        .tool(WebSearch::new().with_ctx(ctx.clone()))
        .tool(RunCmd { ctx: ctx.clone() })
        .build();

    let prompt = render_prompt(history, &user_msg);
    let outcome = stream_helper::drive(
        rig_agent.stream_prompt(prompt).multi_turn(MAX_TURNS),
        agent_kind,
        events,
        pause.as_ref(),
    )
    .await
    .context("chat prompt failed")?;
    let text = match outcome {
        DriveOutcome::Finished { text } => text,
        DriveOutcome::Steered { text, .. } => text,
    };
    if !text.is_empty() {
        let _ = events.send(Event::AgentResponse {
            agent: agent_kind,
            text: text.clone(),
        });
    }
    Ok(text)
}

async fn build_preamble(agent_kind: AgentKind, project: &Project, memory: &Arc<Memory>) -> String {
    let id = match agent_kind {
        AgentKind::Interpreter => identity::INTERPRETER,
        AgentKind::Architect => identity::ARCHITECT,
        AgentKind::Conductor => identity::CONDUCTOR,
    };
    let digest = agents_ctx::memory_digest(memory, 5).await;
    format!(
        "{identity}\n\n\
         {env}\n\
         ## Post-pipeline chat mode\n\
         The Interpret → Build → Run → Reflect pipeline has finished.\n\
         You are now in an interactive chat session with the user so they\n\
         can inspect what happened, ask questions, or ask you to make\n\
         changes. Be concise and concrete. You have your normal tools\n\
         (`files_read`, `files_write`, `memory_search`, `memory_write`,\n\
         `web_search`, `run_cmd`) and can still modify artifacts, notes,\n\
         or source files. The user will re-trigger pipeline phases with\n\
         slash commands (`/interpret`, `/build`, `/run`) when they want\n\
         the pipeline to run again — you don't need to announce a\n\
         sentinel like `LOGIC_WRITTEN` or `BUILD_COMPLETE` in chat.\n\n\
         ## Current project\n\
         - source: `{src}`\n\
         - project root: `{root}`\n\
         - logic.md: `{logic}`\n\
         - runtime.yaml: `{rt}`\n\
         - artifacts dir: `{art}`\n\n\
         {digest}",
        identity = id,
        env = agents_ctx::env_block(),
        src = project.source.display(),
        root = project.root.display(),
        logic = project.logic_md().display(),
        rt = project.runtime_yaml().display(),
        art = project.artifacts_dir().display(),
        digest = digest,
    )
}

fn render_prompt(history: &[ChatTurn], user_msg: &str) -> String {
    if history.is_empty() {
        return user_msg.to_string();
    }
    let mut s = String::new();
    s.push_str("## Chat history so far\n\n");
    for (i, t) in history.iter().enumerate() {
        s.push_str(&format!(
            "### Turn {} — user said:\n{}\n\n### Turn {} — {} replied:\n{}\n\n",
            i + 1,
            t.user,
            i + 1,
            match t.agent {
                AgentKind::Interpreter => "Interpreter",
                AgentKind::Architect => "Architect",
                AgentKind::Conductor => "Conductor",
            },
            t.assistant,
        ));
    }
    s.push_str("## Current user message\n\n");
    s.push_str(user_msg);
    s
}
