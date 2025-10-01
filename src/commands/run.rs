//! `hal run <file>` — execute the artifact bundle, (re)building if the
//! source hash has drifted.

use std::path::Path;
use std::sync::Arc;

use anyhow::Result;

use crate::agents::{conductor, reflector};
use crate::config::Config;
use crate::container;
use crate::control::PauseController;
use crate::events::{Event, Phase};
use crate::llm::{LlmClient, XaiEmbedder};
use crate::memory::Memory;
use crate::pipeline;
use crate::project::Project;
use crate::tui;

pub async fn run(file: &Path, direct: bool, force: bool, args: &[String]) -> Result<()> {
    if !file.exists() {
        anyhow::bail!("source file does not exist: {}", file.display());
    }
    let project = Arc::new(Project::open(file)?);
    project.ensure_layout()?;

    let current_hash = project.source_hash()?;
    let cached_lock = project.read_build_lock()?;
    let cache_hit = matches!(&cached_lock, Some(lock) if lock.source_hash == current_hash);
    // Also require that the artifacts the lock points at actually still
    // exist on disk — a half-deleted bundle must not masquerade as a
    // valid cache.
    let artifacts_ok = project.runtime_yaml().exists() && project.logic_md().exists();
    let needs_build = force || !cache_hit || !artifacts_ok;

    tracing::info!(
        target: "hal::commands::run",
        source = %project.source.display(),
        project_id = %project.id,
        source_hash = %current_hash,
        cache_hit,
        artifacts_ok,
        force,
        needs_build,
        args = ?args,
        "starting `hal run`"
    );

    if needs_build {
        // Wipe any stale bundle so the Architect starts from a clean
        // slate. Without this, partial re-runs can leave half-written
        // scripts from a previous build alongside freshly written ones.
        project.invalidate_build()?;
    }

    let config = Config::load_for_source(file)?;
    let embedder = XaiEmbedder::from_config(&config).ok().map(|e| e.into_arc());
    let memory = Arc::new(Memory::open_with_embedder(&project.memory_db(), embedder).await?);

    let (tx, rx) = pipeline::make_channel();
    let tui_rx = tx.subscribe();
    let log_sink = crate::events::spawn_tracing_sink(tx.subscribe());
    let project_log_sink =
        crate::events::spawn_project_log_sink(project.logs_dir(), tx.subscribe());
    let pause = Arc::new(PauseController::new());

    // Optional event-trace sink for debugging / verifying streaming.
    // Enable with `HAL_TRACE_EVENTS=/path/to/trace.jsonl`.
    let trace = spawn_trace_sink(tx.subscribe());

    let sink = if direct {
        spawn_interrupt_handler(tx.subscribe());
        Some(spawn_stdout_sink(rx))
    } else {
        None
    };

    let mut work = {
        let project = project.clone();
        let memory = memory.clone();
        let tx = tx.clone();
        let config = config.clone();
        let args = args.to_vec();
        let pause = pause.clone();
        tokio::spawn(async move {
            let result = async {
                let needs_llm = needs_build || config.conductor_reflect;
                let llm = if needs_llm {
                    Some(Arc::new(LlmClient::new(config.clone())?))
                } else {
                    None
                };
                if needs_build {
                    pipeline::run_build(
                        project.clone(),
                        memory.clone(),
                        llm.clone().expect("llm present when needs_build"),
                        &tx,
                        Some(pause.clone()),
                        &args,
                    )
                    .await?;
                }
                let r = conductor::run(project.clone(), memory.clone(), &args, &tx).await?;

                // A run the user stopped with Ctrl-C is not worth an LLM
                // turn, and they are waiting for the prompt back.
                if config.conductor_reflect && !container::interrupted() {
                    if let Some(llm) = llm {
                        let summary = reflector::RunSummary {
                            image: r.image.clone(),
                            command: r.command.clone(),
                            cwd: r.cwd.clone(),
                            args: r.args.clone(),
                            exit_code: r.exit_code,
                            run_dir: r.run_dir.clone(),
                            stdout_tail: r.stdout_tail.clone(),
                            stderr_tail: r.stderr_tail.clone(),
                        };
                        let _ = reflector::run(
                            project.clone(),
                            memory.clone(),
                            llm,
                            summary,
                            &tx,
                            config.conductor_reflect_max_turns,
                            Some(pause.clone()),
                        )
                        .await;
                    }
                }
                // Mark all remaining running phases (including Reflecting)
                // as Done so the header no longer shows them counting up.
                let _ = tx.send(crate::events::Event::Phase(crate::events::Phase::Done));
                anyhow::Ok(r.exit_code)
            }
            .await;
            // Tell the TUI. Without this it keeps showing the failed
            // phase as in progress until the user quits.
            if let Err(e) = &result {
                let _ = tx.send(Event::Failed {
                    message: format!("{e:#}"),
                });
            }
            result
        })
    };

    if !direct {
        let runtime_handle = match LlmClient::new(config.clone()) {
            Ok(llm) => Some(tui::app::ChatRuntime {
                project: project.clone(),
                memory: memory.clone(),
                llm: Arc::new(llm),
                config: config.clone(),
                runtime_args: args.to_vec(),
                can_run: true,
            }),
            Err(e) => {
                tracing::warn!(
                    target: "hal::commands::run",
                    error = %e,
                    "chat runtime disabled: LLM client construction failed"
                );
                None
            }
        };
        let _ = tui::run_tui(
            tui_rx,
            Some(pause.clone()),
            tx.clone(),
            project.clone(),
            runtime_handle,
        )
        .await?;
    }

    // The user quit the TUI while the pipeline was still going (e.g. a
    // long-running program). Stop it instead of waiting invisibly.
    if !direct && !work.is_finished() {
        stop_unfinished_work(&mut work).await;
    }

    let exit_code = if work.is_finished() || direct {
        match work.await {
            Ok(r) => r?,
            Err(e) if e.is_cancelled() => 130,
            Err(e) => return Err(e.into()),
        }
    } else {
        130
    };
    drop(tx);
    if let Some(s) = sink {
        let _ = s.await;
    }
    if let Some(t) = trace {
        let _ = t.await;
    }
    let _ = log_sink.await;
    let _ = project_log_sink.await;

    if exit_code != 0 {
        std::process::exit(exit_code);
    }
    Ok(())
}

/// After the TUI closes on an unfinished pipeline: SIGINT any running
/// container, give it the grace period to exit, then abort what is
/// left (LLM turns, chat runs) and force-remove stray containers.
async fn stop_unfinished_work(work: &mut tokio::task::JoinHandle<Result<i32>>) {
    container::interrupt_runs();
    if container::active_runs() > 0 {
        eprintln!("hal: stopping the running container\u{2026}");
        let budget = container::INTERRUPT_GRACE + std::time::Duration::from_secs(5);
        let _ = tokio::time::timeout(budget, async {
            while container::active_runs() > 0 && !work.is_finished() {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        })
        .await;
        // Let the Conductor write meta.json once its container is gone.
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while !work.is_finished() {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        })
        .await;
    }
    if !work.is_finished() {
        work.abort();
        let _ = (&mut *work).await;
    }
    if let Ok(docker) = container::connect() {
        container::remove_all_runs(&docker).await;
    }
}

/// `-d` Ctrl-C handling. While the program itself runs (the Running
/// phase), the first Ctrl-C is forwarded to its container as SIGINT so
/// it can exit cleanly (e.g. print its summary); a second one
/// force-quits. In any other phase (building, reflecting) Ctrl-C quits
/// right away and removes any probe containers.
fn spawn_interrupt_handler(mut events: tokio::sync::broadcast::Receiver<Event>) {
    use tokio::sync::broadcast::error::RecvError;
    tokio::spawn(async move {
        let mut running = false;
        let mut bus_open = true;
        loop {
            tokio::select! {
                evt = events.recv(), if bus_open => match evt {
                    Ok(Event::Phase(p)) => running = matches!(p, Phase::Running),
                    Ok(_) | Err(RecvError::Lagged(_)) => {}
                    Err(RecvError::Closed) => bus_open = false,
                },
                r = tokio::signal::ctrl_c() => {
                    if r.is_err() {
                        return;
                    }
                    break;
                }
            }
        }
        if running && container::active_runs() > 0 {
            eprintln!("\nhal: interrupt; stopping the container (Ctrl-C again to force quit)");
            container::interrupt_runs();
            if tokio::signal::ctrl_c().await.is_err() {
                return;
            }
        }
        if let Ok(docker) = container::connect() {
            container::remove_all_runs(&docker).await;
        }
        std::process::exit(130);
    });
}

fn spawn_stdout_sink(
    mut rx: tokio::sync::broadcast::Receiver<Event>,
) -> tokio::task::JoinHandle<()> {
    // `-d` is "detached": behave like running the script directly.
    // Only the container's own stdout/stderr reach this process's
    // stdout/stderr. Pipeline bookkeeping (phases, agent logs, tool
    // calls, reflector chatter) is suppressed. Errors still surface
    // through the returned error and the process exit code.
    tokio::spawn(async move {
        while let Ok(evt) = rx.recv().await {
            match evt {
                Event::ConductorStdout(s) => {
                    use std::io::Write;
                    let _ = std::io::stdout().write_all(s.as_bytes());
                    let _ = std::io::stdout().flush();
                }
                Event::ConductorStderr(s) => {
                    use std::io::Write;
                    let _ = std::io::stderr().write_all(s.as_bytes());
                    let _ = std::io::stderr().flush();
                }
                // A pipeline failure is also returned from `run`, and
                // main prints it once on exit.
                _ => {}
            }
        }
    })
}

/// Optional debug sink: when `HAL_TRACE_EVENTS=<path>` is set, write one
/// JSON object per event to that file. Useful for verifying end-to-end
/// that streaming token events are actually reaching the bus without
/// having to stare at the TUI.
fn spawn_trace_sink(
    mut rx: tokio::sync::broadcast::Receiver<Event>,
) -> Option<tokio::task::JoinHandle<()>> {
    let path = std::env::var("HAL_TRACE_EVENTS").ok()?;
    Some(tokio::spawn(async move {
        use std::io::Write;
        let f = match std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            Ok(f) => f,
            Err(e) => {
                eprintln!("hal: HAL_TRACE_EVENTS open {path}: {e}");
                return;
            }
        };
        let mut w = std::io::BufWriter::new(f);
        while let Ok(evt) = rx.recv().await {
            if let Ok(s) = serde_json::to_string(&evt) {
                let _ = writeln!(w, "{s}");
                let _ = w.flush();
            }
        }
    }))
}
