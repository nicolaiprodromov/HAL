//! `hal build <file>` — run Interpreter + Architect, produce the artifact
//! bundle and refresh `build.lock`.

use std::path::Path;
use std::sync::Arc;

use anyhow::Result;

use crate::config::Config;
use crate::control::PauseController;
use crate::llm::{LlmClient, XaiEmbedder};
use crate::memory::Memory;
use crate::pipeline;
use crate::project::Project;
use crate::tui;

pub async fn run(file: &Path, direct: bool, force: bool) -> Result<()> {
    if !file.exists() {
        anyhow::bail!("source file does not exist: {}", file.display());
    }
    let project = Arc::new(Project::open(file)?);
    project.ensure_layout()?;

    tracing::info!(
        target: "hal::commands::build",
        source = %project.source.display(),
        project_id = %project.id,
        root = %project.root.display(),
        force,
        direct,
        "starting `hal build`"
    );

    // An explicit `hal build` ALWAYS means "start over": blow away any
    // previously-materialised logic/artifacts so the Architect's
    // `files_write` calls don't interleave with stale outputs. The
    // `force` flag is redundant for `hal build` (which already rebuilds
    // unconditionally) but we honour it for symmetry with `hal run -f`.
    let _ = force;
    project.invalidate_build()?;

    let config = Config::load_for_source(file)?;
    let llm = Arc::new(LlmClient::new(config.clone())?);
    let embedder = XaiEmbedder::from_config(&config).ok().map(|e| e.into_arc());
    let memory = Arc::new(Memory::open_with_embedder(&project.memory_db(), embedder).await?);

    let (tx, rx) = pipeline::make_channel();
    let tui_rx = tx.subscribe();
    let log_sink = crate::events::spawn_tracing_sink(tx.subscribe());
    let project_log_sink =
        crate::events::spawn_project_log_sink(project.logs_dir(), tx.subscribe());
    let pause = Arc::new(PauseController::new());

    let sink = if direct {
        Some(spawn_stdout_sink(rx))
    } else {
        None
    };

    let build_task = {
        let project = project.clone();
        let memory = memory.clone();
        let llm = llm.clone();
        let tx = tx.clone();
        let pause = pause.clone();
        tokio::spawn(async move {
            let r = pipeline::run_build(project, memory, llm, &tx, Some(pause), &[]).await;
            // Signal the TUI that the pipeline is complete so it can
            // unblock its event loop. This mirrors what the Conductor
            // emits on `hal run`.
            match &r {
                Ok(_) => {
                    let _ = tx.send(crate::events::Event::Done { exit_code: 0 });
                }
                Err(e) => {
                    let _ = tx.send(crate::events::Event::Failed {
                        message: format!("{e:#}"),
                    });
                }
            }
            r
        })
    };
    if !direct {
        // TUI drives the display; the build task runs concurrently.
        let tui_tx = tx.clone();
        let runtime = tui::app::ChatRuntime {
            project: project.clone(),
            memory: memory.clone(),
            llm: llm.clone(),
            config: config.clone(),
            runtime_args: Vec::new(),
            can_run: false,
        };
        // Drop the outer sender so the channel closes once the build
        // task finishes (the TUI holds its own clone).
        drop(tx);
        let _ = tui::run_tui(
            tui_rx,
            Some(pause.clone()),
            tui_tx,
            project.clone(),
            Some(runtime),
        )
        .await?;
    } else {
        drop(tx);
    }
    let outcome = build_task.await??;
    if let Some(s) = sink {
        let _ = s.await;
    }
    let _ = log_sink.await;
    let _ = project_log_sink.await;
    let _ = outcome;

    if direct {
        println!("hal build: bundle ready at {}", project.root.display());
    }
    Ok(())
}

fn spawn_stdout_sink(
    mut rx: tokio::sync::broadcast::Receiver<crate::events::Event>,
) -> tokio::task::JoinHandle<()> {
    // Build has no container, so in `-d` mode we only surface failures.
    // A confirmation line is printed by the caller once the task
    // finishes successfully.
    tokio::spawn(async move {
        while let Ok(evt) = rx.recv().await {
            if let crate::events::Event::Failed { message } = evt {
                eprintln!("hal: {message}");
            }
        }
    })
}
