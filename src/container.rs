//! bollard wrapper for the Conductor.
//!
//! Responsibilities:
//! - connect to the local Docker daemon (docker-in-docker in the dev container)
//! - ensure the requested image is present (pull if missing)
//! - run a one-shot command in a fresh container that mirrors the user's
//!   working environment (artifacts mounted read-write, the source's
//!   parent dir mounted at the same absolute path when possible, ports
//!   forwarded 1:1, host network when requested), streaming
//!   stdout/stderr back to the caller via a tokio channel and returning
//!   the final exit code.
//!
//! ## Container philosophy
//!
//! The container is a *containment*, not a cage. By default:
//!
//! - `artifacts/` is mounted **read-write** at `/work`.
//! - When `mirror_workspace` is enabled (the default), the parent of the
//!   source `.hal` file is mounted at the same absolute path inside the
//!   container. The program then runs with the same view of the world
//!   as if it had been invoked directly on the host.
//! - Ports listed in `runtime.yaml` are forwarded to the host (with
//!   `network_mode: host` on Linux, port publishing is a no-op, so we
//!   default to a bridge network and publish each requested port).
//! - Extra mounts, env vars, and privileged flags can be added per
//!   project via `runtime.yaml` so the Architect can tailor the box to
//!   whatever the script actually needs (webapp ports, /dev/fuse, etc.).

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use anyhow::{Context, Result};
use bollard::container::LogOutput;
use bollard::models::{
    ContainerCreateBody, HostConfig, PortBinding, RestartPolicy, RestartPolicyNameEnum,
};
use bollard::query_parameters::{
    CreateContainerOptionsBuilder, CreateImageOptionsBuilder, KillContainerOptionsBuilder,
    LogsOptionsBuilder, RemoveContainerOptionsBuilder, WaitContainerOptionsBuilder,
};
use bollard::Docker;
use futures::StreamExt;
use tokio::sync::{mpsc, watch};

/// Either stdout or stderr bytes from a running container.
#[derive(Debug, Clone)]
pub enum OutputFrame {
    Stdout(String),
    Stderr(String),
}

/// Callback invoked for container-lifecycle log lines (image pull
/// progress, create, start, wait). Used by the Conductor and other
/// agents to surface container construction in the UI.
pub type LifecycleLogger = Arc<dyn Fn(String) + Send + Sync>;

pub struct RunOutcome {
    pub exit_code: i32,
}

/// Describes a bind mount from host → container.
#[derive(Debug, Clone)]
pub struct BindMount {
    pub host_path: String,
    pub container_path: String,
    /// `true` → `ro`, `false` → `rw`.
    pub read_only: bool,
}

/// Describes a port to forward.
#[derive(Debug, Clone)]
pub struct PortSpec {
    pub container_port: u16,
    pub host_port: Option<u16>,
    /// `"tcp"` | `"udp"`.
    pub protocol: String,
}

/// Full runtime knobs that end up on `HostConfig` / `ContainerConfig`.
#[derive(Debug, Clone, Default)]
pub struct ContainerSpec {
    /// Extra bind mounts *in addition* to the artifacts mount.
    pub mounts: Vec<BindMount>,
    /// Ports to publish.
    pub ports: Vec<PortSpec>,
    /// `"bridge"` (default), `"host"`, `"none"`, or a named network.
    pub network_mode: Option<String>,
    /// When true, run the container with `--privileged`. Off by default.
    pub privileged: bool,
    /// When true (default), mount the artifacts directory read-write so
    /// the program can write alongside its bundle (the pattern almost
    /// every non-trivial workload needs).
    pub artifacts_rw: bool,
    /// Extra capabilities to add (e.g. `"NET_ADMIN"`).
    pub cap_add: Vec<String>,
    /// User spec passed to Docker (`"0:0"` by default so the program
    /// can touch /work without dev-container UID mismatches).
    pub user: Option<String>,
    /// Optional extra /etc/hosts entries.
    pub extra_hosts: Vec<String>,
    /// tmpfs mounts (`{container_path: options}`).
    pub tmpfs: HashMap<String, String>,
    /// Shared-memory size in bytes (defaults to Docker's 64MiB).
    pub shm_size: Option<i64>,
    /// Absolute host directory that was the user's working directory
    /// when they invoked `hal`. When set, it is bind-mounted at the
    /// same absolute path RW inside the container *and* becomes the
    /// container's default working directory (unless `cwd_in_work`
    /// overrides). This is what makes relative arguments like
    /// `jobs.txt` resolve to the same file they would on the host.
    pub host_cwd: Option<std::path::PathBuf>,
}

impl ContainerSpec {
    /// A sane default spec that mirrors "running on host" as closely as
    /// possible: artifacts mounted RW, bridge network, no caps dropped.
    pub fn permissive() -> Self {
        Self {
            artifacts_rw: true,
            network_mode: Some("bridge".to_string()),
            user: None,
            ..Default::default()
        }
    }
}

// -- Interrupts -------------------------------------------------------------

/// How long a container gets to exit after SIGINT before it is killed.
pub const INTERRUPT_GRACE: Duration = Duration::from_secs(10);

fn interrupt_flag() -> &'static watch::Sender<bool> {
    static FLAG: OnceLock<watch::Sender<bool>> = OnceLock::new();
    FLAG.get_or_init(|| watch::channel(false).0)
}

fn live_containers() -> &'static Mutex<HashSet<String>> {
    static LIVE: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    LIVE.get_or_init(Default::default)
}

/// Ask every running `run_one_shot` container to stop, the way Ctrl-C
/// stops a program in a terminal: SIGINT to the whole process group,
/// then SIGKILL after [`INTERRUPT_GRACE`]. Runs that have not started
/// yet refuse to start. One-way for the rest of the process.
pub fn interrupt_runs() {
    interrupt_flag().send_replace(true);
}

/// True once [`interrupt_runs`] has been called.
pub fn interrupted() -> bool {
    *interrupt_flag().borrow()
}

/// Number of HAL containers currently created and not yet removed.
pub fn active_runs() -> usize {
    live_containers().lock().map(|s| s.len()).unwrap_or(0)
}

/// Force-remove every live HAL container. Last-resort cleanup before
/// the process exits without waiting for runs to finish.
pub async fn remove_all_runs(docker: &Docker) {
    let names: Vec<String> = live_containers()
        .lock()
        .map(|s| s.iter().cloned().collect())
        .unwrap_or_default();
    for name in names {
        let rm_opts = RemoveContainerOptionsBuilder::default().force(true).build();
        let _ = docker.remove_container(&name, Some(rm_opts)).await;
    }
}

/// Connect to the local Docker daemon using sane defaults (unix socket
/// if present, falling back to environment variables).
pub fn connect() -> Result<Docker> {
    Docker::connect_with_local_defaults().context("connecting to Docker daemon")
}

/// Ensure `image` is present locally, pulling it if necessary.
/// `log` (when provided) receives progress messages suitable for the UI.
pub async fn ensure_image(
    docker: &Docker,
    image: &str,
    log: Option<&LifecycleLogger>,
) -> Result<()> {
    if docker.inspect_image(image).await.is_ok() {
        if let Some(l) = log {
            l(format!("image cached: {image}"));
        }
        return Ok(());
    }
    if let Some(l) = log {
        l(format!("pulling image {image}\u{2026}"));
    }
    let opts = CreateImageOptionsBuilder::default()
        .from_image(image)
        .build();
    let mut stream = docker.create_image(Some(opts), None, None);
    let mut last_status: Option<String> = None;
    while let Some(chunk) = stream.next().await {
        let info = chunk.with_context(|| format!("pulling image {image}"))?;
        if let Some(l) = log {
            if let Some(status) = info.status.as_deref() {
                if last_status.as_deref() != Some(status) {
                    l(format!("pull: {status}"));
                    last_status = Some(status.to_string());
                }
            }
        }
    }
    if let Some(l) = log {
        l(format!("image ready: {image}"));
    }
    Ok(())
}

/// Run `command` inside a fresh container built from `image`. `artifacts_dir`
/// is always mounted at `/work`; `spec` adds whatever extra mounts,
/// ports, and flags the runtime.yaml asked for.
///
/// Output frames are sent on `out`. The function returns when the
/// container has exited, yielding its status code. [`interrupt_runs`]
/// stops it the way Ctrl-C would.
#[allow(clippy::too_many_arguments)]
pub async fn run_one_shot(
    docker: &Docker,
    image: &str,
    artifacts_dir: &Path,
    command: &str,
    cwd_in_work: Option<&str>,
    env: &HashMap<String, String>,
    args: &[String],
    spec: &ContainerSpec,
    out: mpsc::Sender<OutputFrame>,
    log: Option<LifecycleLogger>,
) -> Result<RunOutcome> {
    if interrupted() {
        anyhow::bail!("interrupted; not starting a container");
    }
    run_with_stop(
        docker,
        image,
        artifacts_dir,
        command,
        cwd_in_work,
        env,
        args,
        spec,
        out,
        log,
        interrupt_flag().subscribe(),
    )
    .await
}

/// [`run_one_shot`] with an explicit stop signal: when `stop_rx` turns
/// true the container gets SIGINT, then SIGKILL after the grace period.
#[allow(clippy::too_many_arguments)]
async fn run_with_stop(
    docker: &Docker,
    image: &str,
    artifacts_dir: &Path,
    command: &str,
    cwd_in_work: Option<&str>,
    env: &HashMap<String, String>,
    args: &[String],
    spec: &ContainerSpec,
    out: mpsc::Sender<OutputFrame>,
    log: Option<LifecycleLogger>,
    mut stop_rx: watch::Receiver<bool>,
) -> Result<RunOutcome> {
    ensure_image(docker, image, log.as_ref()).await?;

    let name = format!(
        "hal-run-{}",
        uuid::Uuid::new_v4()
            .simple()
            .to_string()
            .get(..12)
            .unwrap_or("run")
    );

    // Build the bind-mount list. Artifacts is always present; every
    // entry in `spec.mounts` is appended verbatim.
    let artifacts_mode = if spec.artifacts_rw { "rw" } else { "ro" };
    let artifacts_host = artifacts_dir
        .canonicalize()
        .with_context(|| format!("canonicalize {}", artifacts_dir.display()))?;
    let mut binds: Vec<String> = vec![format!(
        "{}:/work:{artifacts_mode}",
        artifacts_host.display()
    )];
    // Track container-side paths we've already bound so we don't emit
    // duplicates (Docker tolerates them, but duplicates confuse logs
    // and sometimes trip up older daemons).
    let mut seen_container_paths: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    seen_container_paths.insert("/work".to_string());

    // If the caller provided a host cwd, mount it RW at the *same*
    // absolute path inside the container. This is the single most
    // important bind for "behave like host": it makes relative paths
    // in arguments (`hal script.hal jobs.txt`) resolve correctly and
    // lets scripts read/write sibling files the user can see.
    let host_cwd_canon: Option<std::path::PathBuf> = match &spec.host_cwd {
        Some(p) => p.canonicalize().ok(),
        None => None,
    };
    if let Some(hc) = host_cwd_canon.as_ref() {
        let s = hc.display().to_string();
        if !seen_container_paths.contains(&s) {
            binds.push(format!("{s}:{s}:rw"));
            seen_container_paths.insert(s);
        }
    }

    for m in &spec.mounts {
        // If a mount targets /work explicitly, skip it — we just made
        // that binding above.
        if seen_container_paths.contains(&m.container_path) {
            continue;
        }
        let mode = if m.read_only { "ro" } else { "rw" };
        binds.push(format!("{}:{}:{mode}", m.host_path, m.container_path));
        seen_container_paths.insert(m.container_path.clone());
    }

    // Port bindings.
    let mut port_bindings: HashMap<String, Option<Vec<PortBinding>>> = HashMap::new();
    let mut exposed_ports: Vec<String> = Vec::new();
    for p in &spec.ports {
        let key = format!("{}/{}", p.container_port, p.protocol);
        let host_port = p
            .host_port
            .map(|n| n.to_string())
            .unwrap_or_else(|| p.container_port.to_string());
        port_bindings.insert(
            key.clone(),
            Some(vec![PortBinding {
                host_ip: Some("0.0.0.0".to_string()),
                host_port: Some(host_port),
            }]),
        );
        exposed_ports.push(key);
    }

    let host_config = HostConfig {
        binds: Some(binds.clone()),
        auto_remove: Some(false),
        network_mode: spec
            .network_mode
            .clone()
            .or_else(|| Some("bridge".to_string())),
        privileged: Some(spec.privileged),
        cap_add: if spec.cap_add.is_empty() {
            None
        } else {
            Some(spec.cap_add.clone())
        },
        port_bindings: if port_bindings.is_empty() {
            None
        } else {
            Some(port_bindings)
        },
        extra_hosts: if spec.extra_hosts.is_empty() {
            None
        } else {
            Some(spec.extra_hosts.clone())
        },
        tmpfs: if spec.tmpfs.is_empty() {
            None
        } else {
            Some(spec.tmpfs.clone())
        },
        shm_size: spec.shm_size,
        init: Some(true),
        // Never restart automatically — HAL runs are one-shot.
        restart_policy: Some(RestartPolicy {
            name: Some(RestartPolicyNameEnum::NO),
            maximum_retry_count: Some(0),
        }),
        ..Default::default()
    };

    // Decide the container's working directory.
    //
    // Priority:
    //   1. `cwd_in_work` — explicit hint from runtime.yaml, interpreted
    //      relative to `/work` (artifacts dir). Keeps the legacy
    //      contract working for bundles that say `cwd: "artifacts"`.
    //   2. `spec.host_cwd` — when no explicit cwd is given, run in the
    //      same host directory the user invoked `hal` from so relative
    //      arguments like `jobs.txt` resolve naturally.
    //   3. Fallback: `/work`.
    let working_dir = match cwd_in_work {
        Some(sub) => {
            // `artifacts/` is mounted at `/work`, so a cwd of "artifacts" or
            // "./artifacts" is redundant — collapse it to "/work".
            let trimmed = sub.trim().trim_start_matches("./").trim_matches('/');
            if trimmed.is_empty()
                || trimmed == "."
                || trimmed == "artifacts"
                || trimmed.starts_with("artifacts/")
            {
                let rest = trimmed
                    .strip_prefix("artifacts")
                    .unwrap_or("")
                    .trim_matches('/');
                if rest.is_empty() {
                    "/work".to_string()
                } else {
                    format!("/work/{rest}")
                }
            } else {
                format!("/work/{trimmed}")
            }
        }
        None => match host_cwd_canon.as_ref() {
            Some(hc) => hc.display().to_string(),
            None => "/work".to_string(),
        },
    };

    let mut env_vec: Vec<String> = env.iter().map(|(k, v)| format!("{k}={v}")).collect();
    // Always expose convenient anchor env vars so scripts and the
    // `runtime.yaml` entrypoint command can reference stable paths
    // regardless of the container's cwd.
    //
    // - `HAL_ARTIFACTS` always points at the artifacts mount (`/work`).
    // - `HAL_HOST_CWD` points at the user's host invocation cwd (mounted
    //   at the same absolute path inside the container) when available.
    env_vec.push("HAL_ARTIFACTS=/work".to_string());
    // With `init`, tini is PID 1. This makes it forward signals to the
    // whole process group, so SIGINT reaches a script blocked in `sleep`
    // just like Ctrl-C in a terminal would.
    env_vec.push("TINI_KILL_PROCESS_GROUP=1".to_string());
    if let Some(hc) = host_cwd_canon.as_ref() {
        env_vec.push(format!("HAL_HOST_CWD={}", hc.display()));
    }

    // Always forward positional arguments via `"$@"` so scripts can read
    // them with `$1 / $2 / sys.argv`. `$0` is set to "hal-script" so
    // error messages aren't confusing.
    let mut cmd_vec: Vec<String> = vec![
        "bash".into(),
        "-lc".into(),
        format!("{command} \"$@\""),
        "hal-script".into(),
    ];
    for a in args {
        cmd_vec.push(a.clone());
    }

    let working_dir_display = working_dir.clone();

    let exposed = if exposed_ports.is_empty() {
        None
    } else {
        Some(exposed_ports)
    };

    let config = ContainerCreateBody {
        image: Some(image.to_string()),
        cmd: Some(cmd_vec),
        working_dir: Some(working_dir),
        env: if env_vec.is_empty() {
            None
        } else {
            Some(env_vec)
        },
        exposed_ports: exposed,
        user: spec.user.clone(),
        attach_stdout: Some(true),
        attach_stderr: Some(true),
        tty: Some(false),
        host_config: Some(host_config),
        ..Default::default()
    };

    if let Some(l) = &log {
        l(format!(
            "creating container {name} (image={image}, cwd={working_dir_display}, mounts={}, ports={}, net={}, priv={})",
            binds.len(),
            spec.ports.len(),
            spec.network_mode.as_deref().unwrap_or("bridge"),
            spec.privileged,
        ));
    }
    tracing::info!(
        target: "hal::container",
        %name, %image, cwd = %working_dir_display,
        binds = ?binds,
        ports = ?spec.ports.iter().map(|p| format!("{}/{}", p.container_port, p.protocol)).collect::<Vec<_>>(),
        network = %spec.network_mode.as_deref().unwrap_or("bridge"),
        privileged = spec.privileged,
        "creating container"
    );
    let create_opts = CreateContainerOptionsBuilder::default().name(&name).build();
    docker
        .create_container(Some(create_opts), config)
        .await
        .context("create_container")?;
    // From here on, an early return or a dropped future (a caller's
    // timeout, Ctrl-C) must not strand the container or the log pump.
    let mut guard = RunGuard::new(docker.clone(), name.clone());

    if let Some(l) = &log {
        l(format!("starting container {name}"));
    }
    docker
        .start_container(&name, None)
        .await
        .context("start_container")?;
    if let Some(l) = &log {
        l(format!("container {name} running: {command}"));
    }

    let logs_opts = LogsOptionsBuilder::default()
        .follow(true)
        .stdout(true)
        .stderr(true)
        .build();
    let mut log_stream = docker.logs(&name, Some(logs_opts));

    let log_tx = out.clone();
    let mut log_task = tokio::spawn(async move {
        while let Some(frame) = log_stream.next().await {
            match frame {
                Ok(LogOutput::StdOut { message }) => {
                    let s = String::from_utf8_lossy(&message).to_string();
                    let _ = log_tx.send(OutputFrame::Stdout(s)).await;
                }
                Ok(LogOutput::StdErr { message }) => {
                    let s = String::from_utf8_lossy(&message).to_string();
                    let _ = log_tx.send(OutputFrame::Stderr(s)).await;
                }
                Ok(LogOutput::Console { message }) => {
                    let s = String::from_utf8_lossy(&message).to_string();
                    let _ = log_tx.send(OutputFrame::Stdout(s)).await;
                }
                Ok(_) => {}
                Err(e) => {
                    let _ = log_tx
                        .send(OutputFrame::Stderr(format!("[hal] log error: {e}")))
                        .await;
                    break;
                }
            }
        }
    });
    guard.log_task = Some(log_task.abort_handle());

    // Wait for the container to exit. An interrupt sends SIGINT and
    // keeps waiting, so the program can shut down cleanly (print its
    // summary, flush files); SIGKILL follows if it takes too long.
    let wait_opts = WaitContainerOptionsBuilder::default().build();
    let mut wait_stream = docker.wait_container(&name, Some(wait_opts));
    let mut kill_at: Option<tokio::time::Instant> = None;
    let mut killed = false;
    let mut exit_code: i32 = -1;
    loop {
        let item = tokio::select! {
            item = wait_stream.next() => item,
            // Wrapped so the non-Send `watch::Ref` is dropped right away.
            _ = async { let _ = stop_rx.wait_for(|v| *v).await; }, if kill_at.is_none() => {
                if let Some(l) = &log {
                    l(format!("interrupt: sending SIGINT to container {name}"));
                }
                let opts = KillContainerOptionsBuilder::new().signal("SIGINT").build();
                let _ = docker.kill_container(&name, Some(opts)).await;
                kill_at = Some(tokio::time::Instant::now() + INTERRUPT_GRACE);
                continue;
            }
            _ = tokio::time::sleep_until(kill_at.unwrap_or_else(tokio::time::Instant::now)),
                if kill_at.is_some() && !killed => {
                if let Some(l) = &log {
                    l(format!("container {name} ignored SIGINT for {INTERRUPT_GRACE:?}; killing"));
                }
                let opts = KillContainerOptionsBuilder::new().signal("SIGKILL").build();
                let _ = docker.kill_container(&name, Some(opts)).await;
                killed = true;
                continue;
            }
        };
        match item {
            None => break,
            Some(Ok(resp)) => exit_code = resp.status_code as i32,
            Some(Err(bollard::errors::Error::DockerContainerWaitError { code, .. })) => {
                exit_code = code as i32;
                break;
            }
            Some(Err(e)) => return Err(e.into()),
        }
    }

    // Ensure the log pump has drained. Bollard's follow:true stream
    // normally closes when the container exits, but some daemon
    // versions leave it open indefinitely; bound the wait so a
    // stuck log pipe can't strand the caller.
    let drain_budget = std::time::Duration::from_millis(2000);
    if tokio::time::timeout(drain_budget, &mut log_task)
        .await
        .is_err()
    {
        log_task.abort();
        let _ = log_task.await;
        if let Some(l) = &log {
            l(format!(
                "container {name} log stream did not close within {:?}; aborted pump",
                drain_budget
            ));
        }
    }

    if let Some(l) = &log {
        l(format!("container {name} exited with code {exit_code}"));
    }
    tracing::info!(target: "hal::container", %name, exit_code, "container exited");

    // Best-effort cleanup. Done inline on the normal path so the
    // container is gone before we return; the guard covers the rest.
    let rm_opts = RemoveContainerOptionsBuilder::default().force(true).build();
    let _ = docker.remove_container(&name, Some(rm_opts)).await;
    guard.disarm();

    Ok(RunOutcome { exit_code })
}

/// Cleans up after [`run_one_shot`] when it does not reach its normal
/// end: aborts the log pump (which holds a clone of the output sender,
/// so callers waiting for the channel to close would hang) and
/// force-removes the container, which also kills it if still running.
/// It also tracks the container in the live set used by
/// [`active_runs`] and [`remove_all_runs`].
struct RunGuard {
    docker: Docker,
    name: String,
    log_task: Option<tokio::task::AbortHandle>,
}

impl RunGuard {
    fn new(docker: Docker, name: String) -> Self {
        if let Ok(mut live) = live_containers().lock() {
            live.insert(name.clone());
        }
        Self {
            docker,
            name,
            log_task: None,
        }
    }

    /// The caller removes the container itself; only untrack it.
    fn disarm(&mut self) {
        if let Ok(mut live) = live_containers().lock() {
            live.remove(&self.name);
        }
        self.name.clear();
    }
}

impl Drop for RunGuard {
    fn drop(&mut self) {
        if let Some(h) = self.log_task.take() {
            h.abort();
        }
        if self.name.is_empty() {
            return;
        }
        if let Ok(mut live) = live_containers().lock() {
            live.remove(&self.name);
        }
        let Ok(rt) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let docker = self.docker.clone();
        let name = std::mem::take(&mut self.name);
        tracing::warn!(target: "hal::container", %name, "run cancelled; removing container");
        rt.spawn(async move {
            let rm_opts = RemoveContainerOptionsBuilder::default().force(true).build();
            let _ = docker.remove_container(&name, Some(rm_opts)).await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bollard::query_parameters::ListContainersOptionsBuilder;

    /// Live Docker smoke test — pulls the default base image (already cached
    /// in the dev container) and runs `echo hal`. Ignored by default because
    /// it requires the Docker daemon; run with `cargo test -- --ignored`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore]
    async fn hello_from_container() {
        let docker = match connect() {
            Ok(d) => d,
            Err(_) => return, // no docker available, skip
        };
        if docker.version().await.is_err() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("marker"), b"x").unwrap();

        let (tx, mut rx) = mpsc::channel::<OutputFrame>(32);
        let outcome = run_one_shot(
            &docker,
            "debian:stable-slim",
            dir.path(),
            "echo hal-ok",
            None,
            &HashMap::new(),
            &[],
            &ContainerSpec::permissive(),
            tx,
            None,
        )
        .await
        .expect("run_one_shot");
        assert_eq!(outcome.exit_code, 0);

        let mut saw = false;
        while let Ok(Some(f)) =
            tokio::time::timeout(std::time::Duration::from_millis(200), rx.recv()).await
        {
            if let OutputFrame::Stdout(s) = f {
                if s.contains("hal-ok") {
                    saw = true;
                }
            }
        }
        assert!(saw, "expected to see hal-ok on stdout");
    }

    /// Regression test for the mount+cwd plumbing that powers
    /// `hal script.hal jobs.txt`. When `host_cwd` is set, the container
    /// must:
    ///   - bind-mount that directory at the same absolute path,
    ///   - default its working directory to that path,
    ///   - resolve relative arg tokens (e.g. `jobs.txt`) against it,
    /// all without the caller doing anything clever. We simulate the
    /// real workflow: a `jobs.txt` sitting next to the invocation cwd,
    /// passed as a positional argument, read by a script that lives
    /// under `artifacts/` — exactly the chron.hal scenario.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore]
    async fn host_cwd_mount_resolves_relative_args() {
        let docker = match connect() {
            Ok(d) => d,
            Err(_) => return,
        };
        if docker.version().await.is_err() {
            return;
        }

        // Host workspace (= user's invocation cwd) with a jobs.txt in it.
        let host = tempfile::tempdir().unwrap();
        std::fs::write(host.path().join("jobs.txt"), b"JOB_ONE\nJOB_TWO\n").unwrap();

        // Artifacts dir lives *outside* host_cwd so we also exercise
        // that the artifacts bind at /work is still visible.
        let artifacts = tempfile::tempdir().unwrap();
        std::fs::write(
            artifacts.path().join("reader.sh"),
            b"#!/bin/bash\nset -e\necho cwd=$PWD\necho host=$HAL_HOST_CWD\necho arts=$HAL_ARTIFACTS\nhead -n1 \"$1\"\n",
        )
        .unwrap();

        let mut spec = ContainerSpec::permissive();
        spec.host_cwd = Some(host.path().canonicalize().unwrap());

        let (tx, mut rx) = mpsc::channel::<OutputFrame>(32);
        let outcome = run_one_shot(
            &docker,
            "debian:stable",
            artifacts.path(),
            "bash $HAL_ARTIFACTS/reader.sh",
            None,
            &HashMap::new(),
            &["jobs.txt".to_string()],
            &spec,
            tx,
            None,
        )
        .await
        .expect("run_one_shot");
        assert_eq!(outcome.exit_code, 0, "script failed");

        let mut buf = String::new();
        while let Ok(Some(frame)) =
            tokio::time::timeout(std::time::Duration::from_millis(200), rx.recv()).await
        {
            match frame {
                OutputFrame::Stdout(s) | OutputFrame::Stderr(s) => buf.push_str(&s),
            }
        }
        let host_canon = host.path().canonicalize().unwrap();
        let host_canon_str = host_canon.display().to_string();
        assert!(
            buf.contains(&format!("cwd={host_canon_str}")),
            "container cwd was not host cwd: {buf}"
        );
        assert!(
            buf.contains(&format!("host={host_canon_str}")),
            "HAL_HOST_CWD not set: {buf}"
        );
        assert!(buf.contains("arts=/work"), "HAL_ARTIFACTS not set: {buf}");
        assert!(
            buf.contains("JOB_ONE"),
            "relative arg `jobs.txt` did not resolve against host cwd: {buf}"
        );
    }

    /// An interrupt must reach a script blocked in `sleep` (SIGINT to the
    /// whole process group, like a terminal Ctrl-C) so its trap runs and
    /// it exits on its own, well inside the grace period.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore]
    async fn interrupt_reaches_sleeping_script() {
        let docker = match connect() {
            Ok(d) => d,
            Err(_) => return,
        };
        if docker.version().await.is_err() {
            return;
        }
        let artifacts = tempfile::tempdir().unwrap();
        std::fs::write(
            artifacts.path().join("loop.sh"),
            b"trap 'echo got-int; exit 0' INT\necho started\nwhile true; do sleep 30; done\n",
        )
        .unwrap();
        let (stop_tx, stop_rx) = watch::channel(false);
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            let _ = stop_tx.send(true);
        });

        let (tx, mut rx) = mpsc::channel::<OutputFrame>(32);
        let started = std::time::Instant::now();
        let outcome = run_with_stop(
            &docker,
            "debian:stable-slim",
            artifacts.path(),
            "bash $HAL_ARTIFACTS/loop.sh",
            None,
            &HashMap::new(),
            &[],
            &ContainerSpec::permissive(),
            tx,
            None,
            stop_rx,
        )
        .await
        .expect("run_with_stop");
        let took = started.elapsed();

        let mut buf = String::new();
        while let Some(OutputFrame::Stdout(s) | OutputFrame::Stderr(s)) = rx.recv().await {
            buf.push_str(&s);
        }
        assert_eq!(outcome.exit_code, 0, "trap should exit 0: {buf}");
        assert!(buf.contains("got-int"), "trap did not run: {buf}");
        assert!(
            took < INTERRUPT_GRACE,
            "exited via SIGKILL, not SIGINT ({took:?})"
        );
    }

    async fn hal_container_names(docker: &Docker) -> std::collections::HashSet<String> {
        let mut filters = HashMap::new();
        filters.insert("name", vec!["hal-run-"]);
        let opts = ListContainersOptionsBuilder::default()
            .all(true)
            .filters(&filters)
            .build();
        docker
            .list_containers(Some(opts))
            .await
            .unwrap_or_default()
            .into_iter()
            .flat_map(|c| c.names.unwrap_or_default())
            .collect()
    }

    /// Regression test for the `run_cmd` hang: when a caller drops the
    /// run future (e.g. its timeout fires), the output channel must close
    /// and the container must be killed and removed. Before the fix the
    /// log pump kept a sender alive, so `run_cmd` waited forever.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore]
    async fn cancelled_run_closes_output_and_removes_container() {
        let docker = match connect() {
            Ok(d) => d,
            Err(_) => return,
        };
        if docker.version().await.is_err() {
            return;
        }
        let before = hal_container_names(&docker).await;
        let artifacts = tempfile::tempdir().unwrap();

        let (tx, mut rx) = mpsc::channel::<OutputFrame>(32);
        let spec = ContainerSpec::permissive();
        let env = HashMap::new();
        let run = run_one_shot(
            &docker,
            "debian:stable-slim",
            artifacts.path(),
            "echo started; sleep 60",
            None,
            &env,
            &[],
            &spec,
            tx,
            None,
        );
        let res = tokio::time::timeout(std::time::Duration::from_secs(3), run).await;
        assert!(res.is_err(), "sleep 60 should have hit the 3s timeout");

        // Every sender must be gone, so the channel closes promptly.
        let drained = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while rx.recv().await.is_some() {}
        })
        .await;
        assert!(drained.is_ok(), "output channel stayed open after cancel");

        // The container is removed in the background; poll for it.
        let mut leaked = Vec::new();
        for _ in 0..20 {
            let now = hal_container_names(&docker).await;
            leaked = now.difference(&before).cloned().collect();
            if leaked.is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
        assert!(
            leaked.is_empty(),
            "container leaked after cancel: {leaked:?}"
        );
    }
}
