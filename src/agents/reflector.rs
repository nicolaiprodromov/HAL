//! Reflector agent.
//!
//! The Conductor is intentionally deterministic (bollard runs the
//! container, captures logs + exit code, and records bare run facts in
//! the `conductor` memory namespace). The Reflector is a first-class LLM
//! agent that runs *after* that, reads the run's artefacts from disk, and
//! writes structured observations and fixes into its own `reflector`
//! memory namespace so that the next Interpreter/Architect cycle can
//! consult them.
//!
//! Without this step the memory loop is short-circuited at runtime:
//! the agents would only ever see their own build-time notes, never the
//! truth of what happened when the container actually ran.

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use rig::client::CompletionClient;
use rig::streaming::StreamingPrompt;

use crate::agents::interpreter::build_steering_followup;
use crate::agents::streaming as stream_helper;
use crate::agents::streaming::DriveOutcome;
use crate::control::PauseController;
use crate::events::{Agent as AgentKind, Event, EventSender};
use crate::identity;
use crate::llm::LlmClient;
use crate::memory::Memory;
use crate::project::Project;
use crate::tools::{
    files::FilesRead, memory as mem_tools, shell::RunCmd, web_search::WebSearch, ToolContext,
};

/// Summary of a completed container run, handed to the Reflector.
#[derive(Debug, Clone)]
pub struct RunSummary {
    pub image: String,
    pub command: String,
    pub cwd: Option<String>,
    pub args: Vec<String>,
    pub exit_code: i32,
    pub run_dir: std::path::PathBuf,
    pub stdout_tail: String,
    pub stderr_tail: String,
}

/// Build the user message the Reflector is prompted with. Extracted so
/// tests can validate it without a live LLM.
pub fn build_user_message(project: &Project, s: &RunSummary) -> String {
    let verdict = if s.exit_code == 0 {
        "SUCCESS"
    } else {
        "FAILURE"
    };
    let stdout_path = s.run_dir.join("stdout.log");
    let stderr_path = s.run_dir.join("stderr.log");
    let meta_path = s.run_dir.join("meta.json");
    let args_line = if s.args.is_empty() {
        "(none)".to_string()
    } else {
        crate::agents::conductor::shell_join(&s.args)
    };
    format!(
        "A container run just finished. Observe it and write durable memory notes.\n\n\
         ## Run summary\n\
         - verdict: **{verdict}** (exit={exit})\n\
         - image: `{image}`\n\
         - command: `{cmd}`\n\
         - cwd: `{cwd}`\n\
         - forwarded args: `{args}`\n\
         - run dir: `{run_dir}`\n\n\
         Logs are on disk (use `files_read` if you need more than the tail):\n\
         - stdout: `{stdout}`\n\
         - stderr: `{stderr}`\n\
         - meta: `{meta}`\n\
         - source `.hal`: `{source}`\n\
         - runtime.yaml: `{runtime}`\n\n\
         ## Stdout tail (last lines)\n```\n{stdout_tail}\n```\n\n\
         ## Stderr tail (last lines)\n```\n{stderr_tail}\n```\n\n\
         ## What to do\n\
         1. Decide what future HAL runs on this project would benefit from knowing.\n\
         2. Call `memory_write` one or more times (notes land in your own `reflector` namespace) with concise, durable observations\n   (what worked, what failed, concrete fixes, image/command gotchas).\n\
         3. If relevant, `memory_search` (default namespace = yours) first to avoid duplicating prior notes.\n\
         4. When done, reply with a single line: `REFLECTION_DONE`.\n",
        verdict = verdict,
        exit = s.exit_code,
        image = s.image,
        cmd = s.command,
        cwd = s.cwd.as_deref().unwrap_or(""),
        args = args_line,
        run_dir = s.run_dir.display(),
        stdout = stdout_path.display(),
        stderr = stderr_path.display(),
        meta = meta_path.display(),
        source = project.source.display(),
        runtime = project.runtime_yaml().display(),
        stdout_tail = s.stdout_tail,
        stderr_tail = s.stderr_tail,
    )
}

/// Read the last `max_bytes` of a file as UTF-8 (lossy). Returns empty
/// string if the file is missing.
pub fn tail_file(path: &Path, max_bytes: usize) -> String {
    let Ok(bytes) = std::fs::read(path) else {
        return String::new();
    };
    let start = bytes.len().saturating_sub(max_bytes);
    String::from_utf8_lossy(&bytes[start..]).to_string()
}

/// Build the full Reflector user message: the basic summary plus
/// inlined `logic.md`, `runtime.yaml`, and an `artifacts/` listing.
///
/// The original [`build_user_message`] is retained above for its unit
/// test; this wrapper stacks additional context on top. Inlining these
/// files is what lets the Reflector actually critique design decisions
/// (image choice, entrypoint, missing artifacts, etc.) instead of only
/// logging runtime noise.
pub fn build_user_message_full(project: &Project, s: &RunSummary) -> String {
    let mut out = build_user_message(project, s);

    out.push_str("\n\n---\n\n");
    out.push_str(&inline_file_section(
        "logic.md",
        &project.logic_md(),
        16 * 1024,
    ));
    out.push_str(&inline_file_section(
        "runtime.yaml",
        &project.runtime_yaml(),
        8 * 1024,
    ));

    // Artifact listing (names + sizes, not contents) gives the LLM
    // grounds to ask for specific reads without guessing file names.
    out.push_str("## artifacts/ listing\n");
    match list_artifacts(&project.artifacts_dir()) {
        Some(listing) if !listing.is_empty() => {
            out.push_str("```\n");
            out.push_str(&listing);
            out.push_str("```\n\n");
        }
        _ => out.push_str("_(empty or unreadable)_\n\n"),
    }

    // Ingest the source file too — the Interpreter saw it, the Architect
    // didn't, and neither did the Reflector until now.
    out.push_str(&inline_file_section(
        ".hal source",
        &project.source,
        8 * 1024,
    ));

    out
}

fn inline_file_section(label: &str, path: &std::path::Path, max_bytes: usize) -> String {
    let Ok(raw) = std::fs::read(path) else {
        return format!("## {label}\n_(not found at {})_\n\n", path.display());
    };
    let full_len = raw.len();
    let slice_end = full_len.min(max_bytes);
    let mut end = slice_end;
    // char boundary guard for the lossy conversion below
    while end > 0 && std::str::from_utf8(&raw[..end]).is_err() {
        end -= 1;
    }
    let body = String::from_utf8_lossy(&raw[..end]).to_string();
    let clipped_note = if full_len > end {
        format!("\n… [{} bytes clipped]\n", full_len - end)
    } else {
        String::new()
    };
    format!(
        "## {label} ({path})\n```\n{body}{clipped}\n```\n\n",
        path = path.display(),
        body = body,
        clipped = clipped_note,
    )
}

fn list_artifacts(dir: &std::path::Path) -> Option<String> {
    if !dir.exists() {
        return None;
    }
    let mut out = String::new();
    let mut rows: Vec<(String, u64)> = Vec::new();
    walk_dir(dir, dir, &mut rows, 0);
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    for (rel, size) in rows.into_iter().take(200) {
        out.push_str(&format!("{rel}  ({size} bytes)\n"));
    }
    Some(out)
}

fn walk_dir(
    base: &std::path::Path,
    cur: &std::path::Path,
    out: &mut Vec<(String, u64)>,
    depth: usize,
) {
    if depth > 4 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(cur) else {
        return;
    };
    for entry in rd.flatten() {
        let path = entry.path();
        if let Ok(rel) = path.strip_prefix(base) {
            if let Ok(meta) = entry.metadata() {
                if meta.is_dir() {
                    walk_dir(base, &path, out, depth + 1);
                } else {
                    out.push((rel.display().to_string(), meta.len()));
                }
            }
        }
    }
}

/// Compose the Reflector's system preamble: its own identity document,
/// the environment block, and the prior-memory digest. The Conductor
/// identity is deliberately not included.
pub fn build_preamble(env: &str, digest: &str) -> String {
    format!("{}\n\n{env}\n{digest}", identity::REFLECTOR)
}

/// Run the Reflector against a completed run. Errors from the LLM are
/// logged and swallowed — a reflection failure should never crash `hal run`.
pub async fn run(
    project: Arc<Project>,
    memory: Arc<Memory>,
    llm: Arc<LlmClient>,
    summary: RunSummary,
    events: &EventSender,
    max_turns: usize,
    pause: Option<Arc<PauseController>>,
) -> Result<()> {
    let _ = events.send(Event::Phase(crate::events::Phase::Reflecting));
    let _ = events.send(Event::AgentLog {
        agent: AgentKind::Reflector,
        message: format!("--> {} …", llm.model),
    });

    let mut ctx = ToolContext::new(project.clone(), memory.clone(), AgentKind::Reflector)
        .with_llm(llm.clone())
        .with_events(events.clone());
    if let Some(p) = pause.clone() {
        ctx = ctx.with_pause(p);
    }

    let digest = super::context::memory_digest(&memory, 5).await;
    let preamble = build_preamble(super::context::env_block(), &digest);

    let user_msg = build_user_message_full(&project, &summary);

    let agent = llm
        .client
        .agent(&llm.model)
        .preamble(&preamble)
        .tool(FilesRead { ctx: ctx.clone() })
        .tool(mem_tools::MemorySearch { ctx: ctx.clone() })
        .tool(mem_tools::MemoryWrite { ctx: ctx.clone() })
        .tool(WebSearch::new().with_ctx(ctx.clone()))
        .tool(RunCmd { ctx: ctx.clone() })
        .build();

    let max_turns = max_turns.max(1);
    let delays_ms = [0u64, 1500, 4000];
    let mut last_err: Option<anyhow::Error> = None;
    let mut current_msg = user_msg;
    for (attempt, delay_ms) in delays_ms.iter().enumerate() {
        if *delay_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(*delay_ms)).await;
        }
        // Inner steering loop: on user steer, rebuild the message and
        // retry immediately (ignoring transient-retry budget).
        let mut steer_rounds = 0;
        let response = loop {
            let outcome = match stream_helper::drive(
                agent
                    .stream_prompt(current_msg.clone())
                    .multi_turn(max_turns),
                AgentKind::Reflector,
                events,
                pause.as_ref(),
            )
            .await
            .context("reflector prompt failed")
            {
                Ok(o) => o,
                Err(e) => {
                    let msg = format!("{e:#}");
                    let transient =
                        msg.contains("503") || msg.contains("502") || msg.contains("timeout");
                    let _ = events.send(Event::AgentLog {
                        agent: AgentKind::Reflector,
                        message: format!(
                            "attempt {}/3 failed (transient={transient}): {}",
                            attempt + 1,
                            msg.chars().take(200).collect::<String>()
                        ),
                    });
                    last_err = Some(e);
                    if !transient {
                        return Ok(());
                    }
                    break String::new();
                }
            };
            match outcome {
                DriveOutcome::Finished { text } => break text,
                DriveOutcome::Steered { text, steers } => {
                    steer_rounds += 1;
                    if steer_rounds > 6 {
                        break text;
                    }
                    current_msg = build_steering_followup(&current_msg, &text, &steers);
                }
            }
        };
        if !response.is_empty() {
            let _ = events.send(Event::AgentResponse {
                agent: AgentKind::Reflector,
                text: response.clone(),
            });
            let _ = events.send(Event::AgentLog {
                agent: AgentKind::Reflector,
                message: format!("done ({})", response.lines().next().unwrap_or("")),
            });
            return Ok(());
        }
    }
    if let Some(e) = last_err {
        let _ = events.send(Event::AgentLog {
            agent: AgentKind::Reflector,
            message: format!("giving up (non-fatal): {e:#}"),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn preamble_uses_reflector_identity_not_conductor() {
        let p = build_preamble("ENV_BLOCK", "DIGEST_BLOCK");
        assert!(p.starts_with(identity::REFLECTOR));
        assert!(p.contains("ENV_BLOCK"));
        assert!(p.contains("DIGEST_BLOCK"));
        assert!(!p.contains(identity::CONDUCTOR));
    }

    #[test]
    fn user_message_contains_key_fields() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("p.hal");
        std::fs::write(&src, b"noop").unwrap();
        let project = Project::open(&src).unwrap();
        project.ensure_layout().unwrap();
        let run_dir = project.new_run_dir().unwrap();
        let s = RunSummary {
            image: "python:3.12-slim".into(),
            command: "python main.py".into(),
            cwd: Some("/work".into()),
            args: vec![],
            exit_code: 0,
            run_dir: run_dir.clone(),
            stdout_tail: "FizzBuzz\n".into(),
            stderr_tail: "".into(),
        };
        let msg = build_user_message(&project, &s);
        assert!(msg.contains("SUCCESS"));
        assert!(msg.contains("exit=0"));
        assert!(msg.contains("python:3.12-slim"));
        assert!(msg.contains("python main.py"));
        assert!(msg.contains("FizzBuzz"));
        assert!(msg.contains("REFLECTION_DONE"));
        assert!(msg.contains(run_dir.to_string_lossy().as_ref()));
    }

    #[test]
    fn failure_verdict_renders() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("p.hal");
        std::fs::write(&src, b"noop").unwrap();
        let project = Project::open(&src).unwrap();
        project.ensure_layout().unwrap();
        let s = RunSummary {
            image: "debian:stable-slim".into(),
            command: "bash run.sh".into(),
            cwd: None,
            args: vec![],
            exit_code: 2,
            run_dir: project.new_run_dir().unwrap(),
            stdout_tail: "".into(),
            stderr_tail: "bash: run.sh: No such file\n".into(),
        };
        let msg = build_user_message(&project, &s);
        assert!(msg.contains("FAILURE"));
        assert!(msg.contains("exit=2"));
        assert!(msg.contains("No such file"));
    }

    #[test]
    fn tail_file_respects_limit() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("log.txt");
        let mut f = std::fs::File::create(&p).unwrap();
        for i in 0..1000 {
            writeln!(f, "line {i}").unwrap();
        }
        drop(f);
        let tail = tail_file(&p, 64);
        assert!(tail.len() <= 64);
        assert!(tail.contains("line 999"));
        assert_eq!(tail_file(&dir.path().join("missing"), 32), "");
    }
}
