//! Conductor — reads `runtime.yaml`, runs the entrypoint inside a
//! Docker container, streams output to the event bus, and records the
//! outcome (exit code + tail of logs) into memory for future runs.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use chrono::Utc;
use serde::Deserialize;
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;

use crate::container::{self, BindMount, ContainerSpec, OutputFrame, PortSpec};
use crate::events::{Agent, Event, EventSender};
use crate::memory::Memory;
use crate::project::Project;

#[derive(Debug, Deserialize)]
struct RuntimeYaml {
    #[serde(default = "default_image")]
    image: String,
    entrypoint: RuntimeEntrypoint,
    #[serde(default)]
    env: HashMap<String, String>,

    // Optional container-customisation fields. All default to permissive
    // "behave like host" behaviour.
    #[serde(default)]
    mounts: Vec<YamlMount>,
    #[serde(default)]
    ports: Vec<YamlPort>,
    #[serde(default)]
    network: Option<String>,
    #[serde(default)]
    privileged: Option<bool>,
    /// When true (default), mount `artifacts/` read-write so the program
    /// can write alongside its bundle.
    #[serde(default)]
    artifacts_rw: Option<bool>,
    /// When true (default), also mount the source `.hal` file's parent
    /// directory at the same absolute path inside the container. This
    /// makes scripts that reference sibling files "just work" with the
    /// same paths they would on the host.
    #[serde(default)]
    mirror_workspace: Option<bool>,
    #[serde(default)]
    cap_add: Vec<String>,
    #[serde(default)]
    user: Option<String>,
    #[serde(default)]
    extra_hosts: Vec<String>,
    #[serde(default)]
    tmpfs: HashMap<String, String>,
    #[serde(default)]
    shm_size: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct RuntimeEntrypoint {
    command: String,
    #[serde(default)]
    cwd: Option<String>,
}

#[derive(Debug, Deserialize)]
struct YamlMount {
    host: String,
    container: String,
    #[serde(default)]
    read_only: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum YamlPort {
    Simple(u16),
    Pair {
        container: u16,
        #[serde(default)]
        host: Option<u16>,
        #[serde(default = "default_proto")]
        protocol: String,
    },
    /// Shorthand like "8080:8080/tcp" or "8080".
    Str(String),
}

fn default_proto() -> String {
    "tcp".to_string()
}

fn default_image() -> String {
    crate::config::DEFAULT_IMAGE.into()
}

pub struct RunResult {
    pub exit_code: i32,
    pub run_dir: PathBuf,
    pub image: String,
    pub command: String,
    pub cwd: Option<String>,
    pub args: Vec<String>,
    /// Last bytes of captured stdout (UTF-8, lossy).
    pub stdout_tail: String,
    /// Last bytes of captured stderr (UTF-8, lossy).
    pub stderr_tail: String,
}

pub async fn run(
    project: Arc<Project>,
    memory: Arc<Memory>,
    args: &[String],
    events: &EventSender,
) -> Result<RunResult> {
    let _ = events.send(Event::Phase(crate::events::Phase::Running));

    let runtime_path = project.runtime_yaml();
    let runtime_str = std::fs::read_to_string(&runtime_path)
        .with_context(|| format!("reading runtime.yaml at {}", runtime_path.display()))?;
    let runtime: RuntimeYaml =
        serde_yaml::from_str(&runtime_str).context("parsing runtime.yaml")?;

    tracing::info!(
        target: "hal::conductor",
        image = %runtime.image,
        command = %runtime.entrypoint.command,
        cwd = ?runtime.entrypoint.cwd,
        env_keys = ?runtime.env.keys().collect::<Vec<_>>(),
        mounts = runtime.mounts.len(),
        ports = runtime.ports.len(),
        network = ?runtime.network,
        privileged = ?runtime.privileged,
        "conductor loaded runtime.yaml"
    );

    // Announce the exact run action to the UI *before* we pull images or
    // start containers, so the user can see how their arguments were
    // forwarded.
    let _ = events.send(Event::ConductorRunAction {
        image: runtime.image.clone(),
        command: runtime.entrypoint.command.clone(),
        cwd: runtime.entrypoint.cwd.clone(),
        args: args.to_vec(),
    });

    let run_dir = project.new_run_dir()?;
    let stdout_path = run_dir.join("stdout.log");
    let stderr_path = run_dir.join("stderr.log");
    let meta_path = run_dir.join("meta.json");

    let docker = container::connect()?;
    let (tx, mut rx) = mpsc::channel::<OutputFrame>(256);

    // Lifecycle logger: surface image pulls, container create/start/exit
    // into the Compiler tab so the user can see container construction
    // progress, not just the program's stdout.
    let lifecycle_events = events.clone();
    let lifecycle: container::LifecycleLogger = std::sync::Arc::new(move |line: String| {
        let _ = lifecycle_events.send(Event::AgentLog {
            agent: Agent::Conductor,
            message: line,
        });
    });

    // Pump frames into run logs + event bus.
    let events_clone = events.clone();
    let stdout_path_clone = stdout_path.clone();
    let stderr_path_clone = stderr_path.clone();
    let pump = tokio::spawn(async move {
        let mut so = tokio::fs::File::create(&stdout_path_clone).await.ok();
        let mut se = tokio::fs::File::create(&stderr_path_clone).await.ok();
        let mut out_tail: Vec<String> = Vec::new();
        let mut err_tail: Vec<String> = Vec::new();
        while let Some(frame) = rx.recv().await {
            match frame {
                OutputFrame::Stdout(s) => {
                    if let Some(f) = so.as_mut() {
                        let _ = f.write_all(s.as_bytes()).await;
                    }
                    let _ = events_clone.send(Event::ConductorStdout(s.clone()));
                    out_tail.push(s);
                    if out_tail.len() > 400 {
                        let drop_n = out_tail.len() - 400;
                        out_tail.drain(0..drop_n);
                    }
                }
                OutputFrame::Stderr(s) => {
                    if let Some(f) = se.as_mut() {
                        let _ = f.write_all(s.as_bytes()).await;
                    }
                    let _ = events_clone.send(Event::ConductorStderr(s.clone()));
                    err_tail.push(s);
                    if err_tail.len() > 400 {
                        let drop_n = err_tail.len() - 400;
                        err_tail.drain(0..drop_n);
                    }
                }
            }
        }
        if let Some(mut f) = so {
            let _ = f.flush().await;
        }
        if let Some(mut f) = se {
            let _ = f.flush().await;
        }
        (out_tail, err_tail)
    });

    let spec = build_spec(&project, &runtime);

    let outcome = container::run_one_shot(
        &docker,
        &runtime.image,
        &project.artifacts_dir(),
        &runtime.entrypoint.command,
        runtime.entrypoint.cwd.as_deref(),
        &runtime.env,
        args,
        &spec,
        tx,
        Some(lifecycle),
    )
    .await;

    // Drop the sender side so pump finishes.
    let (out_tail, err_tail) = pump.await.unwrap_or_default();

    let outcome = outcome?;
    let _ = events.send(Event::Done {
        exit_code: outcome.exit_code,
    });

    // Write meta.json.
    let meta = serde_json::json!({
        "started_at": Utc::now().to_rfc3339(),
        "image": runtime.image,
        "command": runtime.entrypoint.command,
        "cwd": runtime.entrypoint.cwd,
        "args": args,
        "exit_code": outcome.exit_code,
    });
    let _ = std::fs::write(&meta_path, serde_json::to_string_pretty(&meta)?);

    // Record the bare facts of the run in memory. The Reflector will
    // augment this with an LLM-generated observation afterwards.
    let stdout_tail_text = clip_tail(&out_tail.concat(), 2000);
    let stderr_tail_text = clip_tail(&err_tail.concat(), 2000);
    let args_display = if args.is_empty() {
        String::new()
    } else {
        format!(" args={}", shell_join(args))
    };
    let note = format!(
        "run at {} exit={} image={} cmd={}{}\n--- stdout tail ---\n{}\n--- stderr tail ---\n{}",
        meta["started_at"].as_str().unwrap_or(""),
        outcome.exit_code,
        runtime.image,
        runtime.entrypoint.command,
        args_display,
        stdout_tail_text,
        stderr_tail_text,
    );
    let _ = memory.write(Agent::Conductor, &note).await;

    Ok(RunResult {
        exit_code: outcome.exit_code,
        run_dir,
        image: runtime.image,
        command: runtime.entrypoint.command,
        cwd: runtime.entrypoint.cwd,
        args: args.to_vec(),
        stdout_tail: stdout_tail_text,
        stderr_tail: stderr_tail_text,
    })
}

fn build_spec(project: &Project, r: &RuntimeYaml) -> ContainerSpec {
    let mut spec = ContainerSpec::permissive();
    spec.artifacts_rw = r.artifacts_rw.unwrap_or(true);
    spec.privileged = r.privileged.unwrap_or(false);
    if let Some(net) = r.network.clone() {
        spec.network_mode = Some(net);
    }
    spec.cap_add = r.cap_add.clone();
    spec.user = r.user.clone();
    spec.extra_hosts = r.extra_hosts.clone();
    spec.tmpfs = r.tmpfs.clone();
    spec.shm_size = r.shm_size;

    // Host cwd: the directory the user ran `hal` from. Bind-mount it
    // at the same absolute path RW inside the container so relative
    // arguments and sibling files "just work". This is the single
    // most important piece of "behave like host" plumbing — without
    // it, `hal script.hal jobs.txt` would fail because `jobs.txt`
    // lives outside any mount point.
    if let Ok(cwd) = std::env::current_dir() {
        if let Ok(cwd) = cwd.canonicalize() {
            spec.host_cwd = Some(cwd);
        }
    }

    // Workspace mirroring: bind the source's parent directory at the
    // same absolute path inside the container. Most of the time this
    // is already covered by `host_cwd` (the user ran `hal` from at or
    // above the source's directory), so we skip it to avoid redundant
    // mounts. Opt-out via `mirror_workspace: false` in runtime.yaml.
    let mirror = r.mirror_workspace.unwrap_or(true);
    if mirror {
        if let Some(parent) = project.source.parent() {
            if let Ok(parent) = parent.canonicalize() {
                let s = parent.display().to_string();
                let already_covered = match spec.host_cwd.as_ref() {
                    Some(hc) => parent.starts_with(hc) || hc.starts_with(&parent),
                    None => false,
                };
                let collides_with_work = s == "/work";
                let collides = r
                    .mounts
                    .iter()
                    .any(|m| m.container == s || m.container == "/work");
                if !collides_with_work && !collides && !already_covered {
                    spec.mounts.push(BindMount {
                        host_path: s.clone(),
                        container_path: s,
                        read_only: false,
                    });
                }
            }
        }
    }

    for m in &r.mounts {
        spec.mounts.push(BindMount {
            host_path: m.host.clone(),
            container_path: m.container.clone(),
            read_only: m.read_only.unwrap_or(false),
        });
    }

    for p in &r.ports {
        let parsed = match p {
            YamlPort::Simple(n) => PortSpec {
                container_port: *n,
                host_port: Some(*n),
                protocol: "tcp".into(),
            },
            YamlPort::Pair {
                container,
                host,
                protocol,
            } => PortSpec {
                container_port: *container,
                host_port: *host,
                protocol: protocol.clone(),
            },
            YamlPort::Str(s) => parse_port_str(s).unwrap_or(PortSpec {
                container_port: 0,
                host_port: None,
                protocol: "tcp".into(),
            }),
        };
        if parsed.container_port != 0 {
            spec.ports.push(parsed);
        }
    }

    spec
}

fn parse_port_str(s: &str) -> Option<PortSpec> {
    // Formats: "8080", "8080:8080", "8080:9090", "8080/tcp", "8080:8080/udp"
    let (main, proto) = match s.split_once('/') {
        Some((m, p)) => (m, p.to_string()),
        None => (s, "tcp".to_string()),
    };
    let (host, container) = match main.split_once(':') {
        Some((h, c)) => (Some(h), c),
        None => (None, main),
    };
    let container_port: u16 = container.trim().parse().ok()?;
    let host_port: Option<u16> = match host {
        Some(h) => h.trim().parse().ok(),
        None => Some(container_port),
    };
    Some(PortSpec {
        container_port,
        host_port,
        protocol: proto,
    })
}

/// Minimal shell-quote: quote with single quotes if the token contains
/// anything other than `[A-Za-z0-9_./:=@+-]`.
pub(crate) fn shell_join(args: &[String]) -> String {
    args.iter()
        .map(|a| {
            if a.is_empty() {
                "''".to_string()
            } else if a
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "_./:=@+-".contains(c))
            {
                a.clone()
            } else {
                // Escape embedded single quotes by closing+escaping+reopening.
                let escaped = a.replace('\'', "'\\''");
                format!("'{escaped}'")
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn clip_tail(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let skip = s.chars().count() - max_chars;
    s.chars().skip(skip).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_parser_handles_common_forms() {
        let p = parse_port_str("8080").unwrap();
        assert_eq!(p.container_port, 8080);
        assert_eq!(p.host_port, Some(8080));
        assert_eq!(p.protocol, "tcp");

        let p = parse_port_str("8080:9090/udp").unwrap();
        assert_eq!(p.container_port, 9090);
        assert_eq!(p.host_port, Some(8080));
        assert_eq!(p.protocol, "udp");

        assert!(parse_port_str("not-a-port").is_none());
    }
}
