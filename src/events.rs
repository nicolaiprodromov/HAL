//! Agent event bus. All information flowing from the pipeline to the
//! UI (TUI or `-d` direct-mode) goes through this one enum.

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Agent {
    Interpreter,
    Architect,
    Conductor,
}

impl std::fmt::Display for Agent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Agent::Interpreter => f.write_str("Interpreter"),
            Agent::Architect => f.write_str("Architect"),
            Agent::Conductor => f.write_str("Conductor"),
        }
    }
}

/// A single event on the pipeline bus. Everything the UI renders — the
/// "Compiler" flow on the left, the "[Conductor] Output" panel on the
/// right, and the direct-mode script output — is derived from this
/// stream.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Event {
    /// Pipeline phase transition.
    Phase(Phase),

    /// Free-form status line from an agent (`"reading source…"`,
    /// `"--> model-1.…"`, etc.). Rendered dim in the Compiler tab.
    AgentLog {
        agent: Agent,
        message: String,
    },

    /// An agent finished its prompt loop and produced a response. This
    /// is the final model output for that agent's turn.
    AgentResponse {
        agent: Agent,
        text: String,
    },

    /// The agent is about to start (or resume) generating tokens. The
    /// TUI uses this to start the "thinking" animation and to open a
    /// fresh streaming bubble in the timeline.
    AgentThinkingStart {
        agent: Agent,
    },

    /// A streamed token (or short text chunk) from the model. The TUI
    /// appends it to the currently open bubble for that agent.
    AgentToken {
        agent: Agent,
        delta: String,
    },

    /// The agent has stopped streaming for now (either because the turn
    /// is done or because a tool call interrupted the stream). The TUI
    /// closes the current bubble.
    AgentThinkingEnd {
        agent: Agent,
    },

    /// A tool call started. `call_id` is a monotonically-increasing id
    /// we allocate in-process so the matching end event can be paired.
    /// `args_preview` is already truncated / pretty-printed.
    ToolCallStart {
        agent: Agent,
        call_id: u64,
        tool: String,
        args_preview: String,
    },

    /// A tool call finished. `output_preview` is truncated; `ok=false`
    /// means the tool returned an error (the preview is the error msg).
    ToolCallEnd {
        agent: Agent,
        call_id: u64,
        tool: String,
        ok: bool,
        output_preview: String,
        duration_ms: u64,
    },

    /// Raw stdout from the Conductor's Docker run. Drives `-d` output
    /// and the "[Conductor] Output" tab.
    ConductorStdout(String),

    /// Raw stderr from the Conductor's Docker run.
    ConductorStderr(String),

    /// Emitted once, right before the Conductor starts the container.
    /// Carries the exact image + entrypoint + forwarded argv so the UI
    /// can render a "Run action" card showing what is about to execute.
    ConductorRunAction {
        image: String,
        command: String,
        cwd: Option<String>,
        args: Vec<String>,
    },

    Done {
        exit_code: i32,
    },
    Failed {
        message: String,
    },

    /// The user paused the pipeline from the TUI. Everything that
    /// consults [`crate::control::PauseController`] stalls until a
    /// matching [`Event::Resumed`]. Emitted purely for log / UI purposes.
    Paused,
    /// The user resumed after a pause.
    Resumed,
    /// A steering message was injected by the user while paused. The
    /// targeted agent will consume it on resume. Emitted for the UI
    /// transcript; the actual delivery happens via
    /// [`crate::control::PauseController::push_steer`].
    Steering {
        agent: Agent,
        message: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase {
    Interpreting,
    Building,
    Running,
    Reflecting,
    Done,
    Failed,
}

impl std::fmt::Display for Phase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Phase::Interpreting => "Interpreting",
            Phase::Building => "Building",
            Phase::Running => "Running",
            Phase::Reflecting => "Reflecting",
            Phase::Done => "Done",
            Phase::Failed => "Failed",
        };
        f.write_str(s)
    }
}

pub type EventSender = broadcast::Sender<Event>;

pub fn channel(capacity: usize) -> (EventSender, broadcast::Receiver<Event>) {
    broadcast::channel(capacity)
}

/// Allocate a fresh, process-global tool-call id. Monotonic.
pub fn next_call_id() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(1);
    N.fetch_add(1, Ordering::Relaxed)
}

/// Drain the event bus into the `tracing` subscriber so every pipeline
/// event ends up in `~/.hal/logs/hal.log` for post-mortem debugging.
/// Called once per `hal build` / `hal run` invocation.
pub fn spawn_tracing_sink(mut rx: broadcast::Receiver<Event>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(evt) => log_event(&evt),
                Err(broadcast::error::RecvError::Closed) => break,
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(target: "hal::events", dropped = n, "tracing sink lagged");
                }
            }
        }
    })
}

fn log_event(evt: &Event) {
    // Be conservative with token-level events: they fire thousands of
    // times per turn and would drown the file log. Demote them to TRACE
    // so they're only recorded when the user asks for `HAL_LOG=trace`.
    match evt {
        Event::Phase(p) => {
            tracing::info!(target: "hal::event", phase = %p, "phase transition");
        }
        Event::AgentLog { agent, message } => {
            tracing::info!(target: "hal::event", %agent, msg = %message, "agent log");
        }
        Event::AgentResponse { agent, text } => {
            let preview: String = text.chars().take(400).collect();
            tracing::info!(target: "hal::event", %agent, response = %preview, "agent response");
        }
        Event::AgentThinkingStart { agent } => {
            tracing::debug!(target: "hal::event", %agent, "agent thinking start");
        }
        Event::AgentThinkingEnd { agent } => {
            tracing::debug!(target: "hal::event", %agent, "agent thinking end");
        }
        Event::AgentToken { agent, delta } => {
            tracing::trace!(target: "hal::event", %agent, delta_len = delta.len(), "agent token");
        }
        Event::ToolCallStart {
            agent,
            call_id,
            tool,
            args_preview,
        } => {
            tracing::info!(
                target: "hal::event",
                %agent, call_id, %tool, args = %args_preview,
                "tool call start"
            );
        }
        Event::ToolCallEnd {
            agent,
            call_id,
            tool,
            ok,
            output_preview,
            duration_ms,
        } => {
            if *ok {
                tracing::info!(
                    target: "hal::event",
                    %agent, call_id, %tool, duration_ms,
                    output = %output_preview,
                    "tool call ok"
                );
            } else {
                tracing::warn!(
                    target: "hal::event",
                    %agent, call_id, %tool, duration_ms,
                    error = %output_preview,
                    "tool call failed"
                );
            }
        }
        Event::ConductorStdout(s) => {
            tracing::debug!(target: "hal::event", bytes = s.len(), "container stdout");
        }
        Event::ConductorStderr(s) => {
            tracing::debug!(target: "hal::event", bytes = s.len(), "container stderr");
        }
        Event::ConductorRunAction {
            image,
            command,
            cwd,
            args,
        } => {
            tracing::info!(
                target: "hal::event",
                %image,
                %command,
                cwd = ?cwd,
                args = ?args,
                "conductor run action"
            );
        }
        Event::Done { exit_code } => {
            tracing::info!(target: "hal::event", exit_code, "pipeline done");
        }
        Event::Failed { message } => {
            tracing::error!(target: "hal::event", %message, "pipeline failed");
        }
        Event::Paused => {
            tracing::info!(target: "hal::event", "pipeline paused by user");
        }
        Event::Resumed => {
            tracing::info!(target: "hal::event", "pipeline resumed by user");
        }
        Event::Steering { agent, message } => {
            let preview: String = message.chars().take(400).collect();
            tracing::info!(target: "hal::event", %agent, steer = %preview, "user steering");
        }
    }
}

/// Spawn a per-project log sink. Writes two files under
/// `<project>/.hal/<id>/logs/`:
/// - `trajectory.jsonl`: one serialised [`Event`] per line (skip token
///   fragments to keep the file sane — tokens are also logged via
///   tracing at TRACE level).
/// - `hal.log`: human-readable timeline (one line per non-token event)
///   with an RFC3339 timestamp prefix.
///
/// The sink lives for the duration of the receiver (until the bus
/// closes). The returned handle should be `.await`ed at shutdown so the
/// last lines make it to disk.
pub fn spawn_project_log_sink(
    logs_dir: std::path::PathBuf,
    mut rx: broadcast::Receiver<Event>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        use std::io::Write;
        if let Err(e) = std::fs::create_dir_all(&logs_dir) {
            tracing::warn!(target: "hal::events", dir = %logs_dir.display(), %e, "cannot create project logs dir");
            return;
        }
        let jsonl_path = logs_dir.join("trajectory.jsonl");
        let human_path = logs_dir.join("hal.log");
        let mut jsonl = match std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&jsonl_path)
        {
            Ok(f) => std::io::BufWriter::new(f),
            Err(e) => {
                tracing::warn!(target: "hal::events", %e, "cannot open trajectory.jsonl");
                return;
            }
        };
        let mut human = match std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&human_path)
        {
            Ok(f) => std::io::BufWriter::new(f),
            Err(e) => {
                tracing::warn!(target: "hal::events", %e, "cannot open project hal.log");
                return;
            }
        };
        let _ = writeln!(
            human,
            "\n──── hal session start @ {} ────",
            chrono::Utc::now().to_rfc3339()
        );
        let _ = human.flush();

        loop {
            match rx.recv().await {
                Ok(evt) => {
                    if let Ok(s) = serde_json::to_string(&evt) {
                        let _ = writeln!(jsonl, "{s}");
                    }
                    if let Some(line) = human_line(&evt) {
                        let ts = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ");
                        let _ = writeln!(human, "{ts} {line}");
                    }
                    let _ = jsonl.flush();
                    let _ = human.flush();
                }
                Err(broadcast::error::RecvError::Closed) => break,
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    let ts = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ");
                    let _ = writeln!(human, "{ts} WARN project log lagged — dropped {n} events");
                }
            }
        }
    })
}

fn human_line(evt: &Event) -> Option<String> {
    match evt {
        Event::Phase(p) => Some(format!("PHASE        {p}")),
        Event::AgentLog { agent, message } => {
            Some(format!("LOG      {:<12} {message}", agent.to_string()))
        }
        Event::AgentResponse { agent, text } => Some(format!(
            "RESPONSE {:<12} {}",
            agent.to_string(),
            one_line(text, 2000)
        )),
        Event::AgentThinkingStart { agent } => Some(format!("THINK-BEG {agent}")),
        Event::AgentThinkingEnd { agent } => Some(format!("THINK-END {agent}")),
        // Individual tokens are too spammy; they go into trajectory.jsonl only.
        Event::AgentToken { .. } => None,
        Event::ToolCallStart {
            agent,
            call_id,
            tool,
            args_preview,
        } => Some(format!(
            "TOOL-BEG {:<12} #{call_id} {tool}({})",
            agent.to_string(),
            one_line(args_preview, 4000)
        )),
        Event::ToolCallEnd {
            agent,
            call_id,
            tool,
            ok,
            output_preview,
            duration_ms,
        } => Some(format!(
            "TOOL-END {:<12} #{call_id} {tool} {} {duration_ms}ms :: {}",
            agent.to_string(),
            if *ok { "OK" } else { "ERR" },
            one_line(output_preview, 4000)
        )),
        Event::ConductorStdout(s) => Some(format!("OUT          {}", one_line(s, 2000))),
        Event::ConductorStderr(s) => Some(format!("ERR          {}", one_line(s, 2000))),
        Event::ConductorRunAction {
            image,
            command,
            cwd,
            args,
        } => Some(format!(
            "RUN-ACTION   image={image} cmd={command} cwd={cwd:?} args={args:?}"
        )),
        Event::Done { exit_code } => Some(format!("DONE         exit={exit_code}")),
        Event::Failed { message } => Some(format!("FAILED       {}", one_line(message, 2000))),
        Event::Paused => Some("PAUSED".into()),
        Event::Resumed => Some("RESUMED".into()),
        Event::Steering { agent, message } => Some(format!(
            "STEER    {:<12} {}",
            agent.to_string(),
            one_line(message, 2000)
        )),
    }
}

fn one_line(s: &str, max: usize) -> String {
    let trimmed: String = s.chars().take(max).collect();
    let single = trimmed.replace('\n', "⏎ ").replace('\r', "");
    if s.chars().count() > max {
        format!("{single}…")
    } else {
        single
    }
}
