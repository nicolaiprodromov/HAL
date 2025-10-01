//! Live end-to-end test against xAI/Grok. Ignored by default — run with:
//!
//!     cargo test --test live_xai -- --ignored --nocapture
//!
//! Requires `MAPERTUI_XAI_TOKEN` in the environment (loaded from `HAL/.env`
//! automatically via dotenvy).

use std::sync::Arc;

use hal::config::Config;
use hal::events::{Agent, Event, Phase};
use hal::llm::{Embedder, LlmClient, XaiEmbedder};
use hal::memory::Memory;
use hal::pipeline;
use hal::project::Project;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore]
async fn fizzbuzz_builds_end_to_end() {
    let _ = dotenvy::dotenv();
    let _ = dotenvy::from_filename("HAL/.env");
    let _ = dotenvy::from_filename(".env");

    // Copy example into a temp dir so we don't pollute the repo's .hal.
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("fizzbuzz.hal");
    std::fs::copy("examples/fizzbuzz.hal", &src).unwrap();

    let project = Arc::new(Project::open(&src).unwrap());
    project.ensure_layout().unwrap();

    let config = Config::from_env();
    assert!(
        config
            .xai_api_key
            .as_deref()
            .map(|s| !s.is_empty())
            .unwrap_or(false),
        "MAPERTUI_XAI_TOKEN must be set in env or HAL/.env for this test"
    );
    let llm = Arc::new(LlmClient::new(config).expect("build llm client"));
    let memory = Arc::new(Memory::open(&project.memory_db()).await.unwrap());

    let (tx, mut rx) = pipeline::make_channel();

    // Stream events to stderr so `--nocapture` shows progress.
    let sink = tokio::spawn(async move {
        let mut phase = None;
        while let Ok(e) = rx.recv().await {
            match e {
                Event::Phase(p) => {
                    phase = Some(p);
                    eprintln!("[phase] {p:?}");
                }
                Event::AgentLog { agent, message } => {
                    // Trim loud tool-output noise.
                    let preview: String = message.chars().take(200).collect();
                    eprintln!("[{agent}] {preview}");
                }
                Event::ToolCallStart {
                    agent,
                    tool,
                    args_preview,
                    ..
                } => {
                    eprintln!("[{agent}] tool={tool} start args={args_preview}");
                }
                Event::ToolCallEnd {
                    agent,
                    tool,
                    ok,
                    output_preview,
                    ..
                } => {
                    eprintln!(
                        "[{agent}] tool={tool} {} result={output_preview}",
                        if ok { "ok" } else { "err" }
                    );
                }
                _ => {}
            }
        }
        phase
    });

    let outcome = pipeline::run_build(project.clone(), memory, llm, &tx, None, &[]).await;
    drop(tx);
    let last_phase = sink.await.unwrap_or(None);

    outcome.expect("pipeline build failed");

    // logic.md + runtime.yaml + build.lock all exist.
    assert!(project.logic_md().is_file(), "logic.md missing");
    assert!(project.runtime_yaml().is_file(), "runtime.yaml missing");
    assert!(project.build_lock().is_file(), "build.lock missing");
    assert!(project.artifacts_dir().is_dir(), "artifacts dir missing");

    // runtime.yaml is valid YAML with the required keys.
    let rt = std::fs::read_to_string(project.runtime_yaml()).unwrap();
    let y: serde_yaml::Value = serde_yaml::from_str(&rt).expect("runtime.yaml parses as YAML");
    assert!(y.get("image").is_some(), "runtime.yaml missing `image`");
    assert!(
        y.get("entrypoint").and_then(|e| e.get("command")).is_some(),
        "runtime.yaml missing `entrypoint.command`"
    );

    // Interpreter+Architect phases were observed.
    assert!(
        matches!(
            last_phase,
            Some(Phase::Building) | Some(Phase::Interpreting)
        ),
        "expected at least Interpreting phase; got {last_phase:?}"
    );
}

fn load_env() {
    let _ = dotenvy::dotenv();
    let _ = dotenvy::from_filename("HAL/.env");
    let _ = dotenvy::from_filename(".env");
}

fn require_key(c: &Config) {
    assert!(
        c.xai_api_key
            .as_deref()
            .map(|s| !s.is_empty())
            .unwrap_or(false),
        "MAPERTUI_XAI_TOKEN must be set in env or HAL/.env for this test"
    );
}

/// Hits `/v1/embeddings` with the default large model and verifies that:
///  - the vector comes back at the advertised dimensionality
///  - semantically similar text ranks above unrelated text via cosine
///  - `Memory` wired to the real embedder returns the similar note first
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore]
async fn xai_embeddings_and_vector_memory() {
    load_env();
    let config = Config::from_env();
    require_key(&config);

    let embedder = XaiEmbedder::from_config(&config).expect("build embedder");
    // Dimensionality check.
    let v = embedder
        .embed(&["hello world".into()])
        .await
        .expect("embed ok");
    assert_eq!(v.len(), 1);
    assert_eq!(
        v[0].len(),
        config.embedding_dims,
        "returned dims must match config"
    );

    // Cosine sanity: near-duplicates >> unrelated.
    let batch = embedder
        .embed(&[
            "print fizzbuzz in bash".into(),
            "bash script printing fizzbuzz".into(),
            "render a fractal with ray marching on the gpu".into(),
        ])
        .await
        .expect("batch embed");
    fn cos(a: &[f32], b: &[f32]) -> f32 {
        let mut d = 0.0f32;
        let mut na = 0.0f32;
        let mut nb = 0.0f32;
        for i in 0..a.len() {
            d += a[i] * b[i];
            na += a[i] * a[i];
            nb += b[i] * b[i];
        }
        d / (na.sqrt() * nb.sqrt())
    }
    let near = cos(&batch[0], &batch[1]);
    let far = cos(&batch[0], &batch[2]);
    assert!(near > far, "near={near} should exceed far={far}");

    // End-to-end: vector memory ranks the semantic match first.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mem.sqlite");
    let arc_emb: Arc<dyn Embedder> = Arc::new(embedder);
    let mem = Memory::open_with_embedder(&path, Some(arc_emb))
        .await
        .unwrap();
    mem.write(
        Agent::Architect,
        "prefer bash for small print-and-sleep scripts",
    )
    .await
    .unwrap();
    mem.write(Agent::Architect, "use numpy for tensor math")
        .await
        .unwrap();
    mem.write(Agent::Architect, "debian:stable-slim pulls fast")
        .await
        .unwrap();
    let hits = mem
        .search(Agent::Architect, "bash script to print numbers", 3)
        .await
        .unwrap();
    assert!(!hits.is_empty());
    assert!(
        hits[0].content.contains("bash"),
        "top hit: {:?}",
        hits[0].content
    );
    assert!(hits[0].score.unwrap() > 0.0);
}

/// End-to-end: build FizzBuzz, run it in Docker, let the Reflector observe
/// the run, and verify a new LLM-authored note landed in Conductor memory.
/// Requires both MAPERTUI_XAI_TOKEN and a working Docker daemon.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore]
async fn reflector_writes_memory_after_container_run() {
    load_env();
    let config = Config::load_for_source(std::path::Path::new("examples/fizzbuzz.hal"))
        .expect("load config");
    require_key(&config);

    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("fizzbuzz.hal");
    std::fs::copy("examples/fizzbuzz.hal", &src).unwrap();

    let project = Arc::new(Project::open(&src).unwrap());
    project.ensure_layout().unwrap();

    let llm = Arc::new(LlmClient::new(config.clone()).unwrap());
    let embedder = XaiEmbedder::from_config(&config).ok().map(|e| e.into_arc());
    let memory = Arc::new(
        Memory::open_with_embedder(&project.memory_db(), embedder)
            .await
            .unwrap(),
    );

    let (tx, mut rx) = pipeline::make_channel();
    let sink = tokio::spawn(async move {
        while let Ok(e) = rx.recv().await {
            if let Event::AgentLog { agent, message } = e {
                let preview: String = message.chars().take(160).collect();
                eprintln!("[{agent}] {preview}");
            }
        }
    });

    pipeline::run_build(project.clone(), memory.clone(), llm.clone(), &tx, None, &[])
        .await
        .expect("build failed");

    let before = memory.count(Agent::Conductor).await.unwrap();

    let r = hal::agents::conductor::run(project.clone(), memory.clone(), &[], &tx)
        .await
        .expect("conductor run");
    assert_eq!(r.exit_code, 0, "fizzbuzz should exit 0");

    let after_run = memory.count(Agent::Conductor).await.unwrap();
    assert!(
        after_run > before,
        "deterministic runner should have written its own note"
    );

    let summary = hal::agents::reflector::RunSummary {
        image: r.image.clone(),
        command: r.command.clone(),
        cwd: r.cwd.clone(),
        args: r.args.clone(),
        exit_code: r.exit_code,
        run_dir: r.run_dir.clone(),
        stdout_tail: r.stdout_tail.clone(),
        stderr_tail: r.stderr_tail.clone(),
    };
    hal::agents::reflector::run(
        project.clone(),
        memory.clone(),
        llm.clone(),
        summary,
        &tx,
        config.conductor_reflect_max_turns,
        None,
    )
    .await
    .expect("reflector run");

    drop(tx);
    let _ = sink.await;

    let after_reflect = memory.count(Agent::Conductor).await.unwrap();
    assert!(
        after_reflect > after_run,
        "reflector should have added at least one note (before={after_run}, after={after_reflect})"
    );

    // And that note must be retrievable via vector search.
    let hits = memory
        .search(Agent::Conductor, "fizzbuzz image command", 5)
        .await
        .unwrap();
    assert!(!hits.is_empty(), "reflector notes should be searchable");
    // Silence unused warnings on Phase import when this is the only test compiled.
    let _ = Phase::Done;
}
