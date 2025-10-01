//! `run_cmd` tool — execute a shell command inside a one-shot
//! container of the caller's choice of image.
//!
//! The single most effective antidote to agents hallucinating about
//! what is and isn't in a given Docker image is letting them just
//! *check*. With this tool the Architect can confirm that
//! `python:3.12-slim` ships with `pip`, that `node:20-slim` has
//! `openssl`, or that `/work/chron.sh` actually exists before it
//! bets a whole `runtime.yaml` on a guess. The Reflector uses it to
//! reproduce failures in-place (re-run the same command, inspect
//! `dpkg -l`, etc.) instead of speculating from stderr tails alone.
//!
//! Safety rails:
//! - Bounded wall-clock timeout (default 30s, hard cap 120s).
//! - Combined stdout/stderr captured up to 16 KiB, the rest clipped.
//! - Container is auto-removed after each call.
//! - Runs with the same "permissive, behave like host" spec the
//!   Conductor uses: host cwd and artifacts bind-mounted at their
//!   real paths, RW, so path-sensitive probes ("does /work/foo.py
//!   exist?") give meaningful answers.

use std::collections::HashMap;
use std::time::Duration;

use rig::completion::ToolDefinition;
use rig::tool::Tool;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use super::ToolContext;
use crate::container::{self, ContainerSpec, OutputFrame};

const DEFAULT_TIMEOUT_SECS: u64 = 30;
const MAX_TIMEOUT_SECS: u64 = 120;
const MAX_CAPTURE_BYTES: usize = 16 * 1024;

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct RunCmdError(String);

impl From<anyhow::Error> for RunCmdError {
    fn from(e: anyhow::Error) -> Self {
        Self(format!("{e:#}"))
    }
}

#[derive(Debug, Deserialize)]
pub struct RunCmdArgs {
    /// Shell command to run. Interpreted by `bash -lc`.
    pub command: String,
    /// Docker image to run the command in. Defaults to the project's
    /// configured default image.
    #[serde(default)]
    pub image: Option<String>,
    /// Wall-clock timeout in seconds (default 30, max 120).
    #[serde(default)]
    pub timeout_secs: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct RunCmdOut {
    pub image: String,
    pub command: String,
    pub exit_code: i32,
    /// `true` when the command was killed because it exceeded
    /// `timeout_secs`. In that case `exit_code` is `-1`.
    pub timed_out: bool,
    /// Combined stdout + stderr, clipped to 16 KiB. Each line is
    /// prefixed with `O` (stdout) or `E` (stderr) so the agent can
    /// tell them apart without a second round trip.
    pub output: String,
}

#[derive(Clone)]
pub struct RunCmd {
    pub ctx: ToolContext,
}

impl Tool for RunCmd {
    const NAME: &'static str = "run_cmd";
    type Error = RunCmdError;
    type Args = RunCmdArgs;
    type Output = RunCmdOut;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.into(),
            description:
                "Run a shell command inside a one-shot Docker container and return its exit \
                 code and combined stdout+stderr (clipped to 16 KiB). Use this to VERIFY \
                 assumptions about images, tools, paths, file contents, and runtime behaviour \
                 instead of guessing. The container is created with the same permissive \
                 mounts as the real runtime (host cwd and artifacts bind-mounted at their \
                 real paths), so path-sensitive probes are meaningful. Bounded by a \
                 wall-clock timeout (default 30s, max 120s)."
                    .into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "Shell command; interpreted by `bash -lc`."
                    },
                    "image": {
                        "type": "string",
                        "description": "Docker image. Defaults to the project's default image."
                    },
                    "timeout_secs": {
                        "type": "integer",
                        "description": "Wall-clock timeout in seconds (default 30, max 120)."
                    }
                },
                "required": ["command"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let image = args
            .image
            .clone()
            .unwrap_or_else(|| crate::config::DEFAULT_IMAGE.to_string());
        let timeout_secs = args
            .timeout_secs
            .unwrap_or(DEFAULT_TIMEOUT_SECS)
            .clamp(1, MAX_TIMEOUT_SECS);

        let guard = self.ctx.begin_call(
            Self::NAME,
            format!(
                "image={} timeout={}s\ncommand=\n{}",
                image, timeout_secs, args.command
            ),
        );

        let docker = match container::connect() {
            Ok(d) => d,
            Err(e) => {
                let msg = format!("docker connect: {e:#}");
                guard.finish_err(msg.clone());
                return Err(RunCmdError(msg));
            }
        };

        let mut spec = ContainerSpec::permissive();
        if let Ok(cwd) = std::env::current_dir().and_then(|p| p.canonicalize()) {
            spec.host_cwd = Some(cwd);
        }

        let (tx, mut rx) = mpsc::channel::<OutputFrame>(128);
        let artifacts_dir = self.ctx.project.artifacts_dir();

        let capture_task = tokio::spawn(async move {
            let mut buf = String::new();
            let mut total = 0usize;
            while let Some(frame) = rx.recv().await {
                let (prefix, s) = match frame {
                    OutputFrame::Stdout(s) => ("O ", s),
                    OutputFrame::Stderr(s) => ("E ", s),
                };
                for line in s.split_inclusive('\n') {
                    if total >= MAX_CAPTURE_BYTES {
                        break;
                    }
                    buf.push_str(prefix);
                    buf.push_str(line);
                    total += prefix.len() + line.len();
                }
                if total >= MAX_CAPTURE_BYTES {
                    buf.push_str("\n… [output clipped at 16 KiB]\n");
                    break;
                }
            }
            // Drain any remaining frames so the sender doesn't block.
            while rx.recv().await.is_some() {}
            buf
        });

        let empty_env: HashMap<String, String> = HashMap::new();
        let empty_args: Vec<String> = Vec::new();
        let run_fut = container::run_one_shot(
            &docker,
            &image,
            &artifacts_dir,
            &args.command,
            None, // default to host_cwd
            &empty_env,
            &empty_args,
            &spec,
            tx,
            None,
        );

        let outcome = tokio::time::timeout(Duration::from_secs(timeout_secs), run_fut).await;
        let (exit_code, timed_out) = match outcome {
            Ok(Ok(o)) => (o.exit_code, false),
            Ok(Err(e)) => {
                let msg = format!("{e:#}");
                guard.finish_err(msg.clone());
                // Let the capture task drain and exit.
                let _ = capture_task.await;
                return Err(RunCmdError(msg));
            }
            Err(_) => (-1, true),
        };

        let output = capture_task.await.unwrap_or_default();

        guard.finish_ok(format!(
            "exit={} timed_out={} bytes={}",
            exit_code,
            timed_out,
            output.len()
        ));

        Ok(RunCmdOut {
            image,
            command: args.command,
            exit_code,
            timed_out,
            output,
        })
    }
}
