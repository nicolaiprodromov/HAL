# HAL

HAL is a single Rust crate: library `src/lib.rs` and binary `src/main.rs` (`hal`). A `.hal` file is natural-language pseudo-code. The runtime runs it through four agents: Interpreter → Architect → Conductor → Reflector. Interpreter, Architect and Reflector are LLM agents (rig-core against the xAI API). The Conductor is deterministic Rust that starts a container with bollard. The Reflector runs after the container exits and writes observations and fixes into its own project-memory namespace (`interpreter`, `architect`, `conductor` = bare run facts, `reflector`).

This file is how to work in the repo. The `justfile` is the command interface. When the README and the justfile disagree, follow the justfile.

## Use `just`

Run `just` to list recipes. Use a recipe whenever one exists. Drop to raw `cargo` only for a single test filter, or for the ignored Docker tests in `src/container.rs` (no recipe wraps those).

Day-to-day recipes compile the **debug** binary. Release recipes are for an optimized binary or an install.

Every recipe shares this crate's `target/` directory. Leave that cache in place. It is why a small change rebuilds in seconds.

### Build and install

| Recipe | What it does |
|---|---|
| `just build` | `cargo build --bin hal` |
| `just release` | `cargo build --release --locked --bin hal` |
| `just install` | `./install.sh`: release build in `target/`, then copy to `~/.cargo/bin` |
| `just install-system` | `./install.sh --system`: copy to `/usr/local/bin` (needs sudo) |

`just install` and `./install.sh` are the same command. The script builds with `--locked` inside the workspace `target/`, then installs that binary. When `CARGO_BUILD_JOBS` is unset it uses `nproc - 1`.

Install only through those recipes. `cargo install --path .` compiles in a throwaway target directory and rebuilds native deps (bundled sqlite, blake3, OpenSSL) from scratch on every run.

### Run a `.hal` file

| Recipe | What it does |
|---|---|
| `just run FILE [ARGS...]` | Debug binary, TUI. Example: `just run examples/chron.hal examples/jobs.txt` |
| `just run-direct FILE [ARGS...]` | Debug binary, detached (`-d`): container stdout/stderr only |
| `just build-hal FILE` | Debug `hal build`: Interpreter + Architect, no container |
| `just run-release FILE [ARGS...]` | `run`, on the release binary |
| `just run-direct-release FILE [ARGS...]` | `run-direct`, on the release binary |

`hal build` calls the model, so it needs `HAL_API_TOKEN`. `hal run` also needs a running Docker daemon. The default image is `debian:stable` (`DEFAULT_IMAGE` in `src/config.rs`).

### Tests and quality

| Recipe | What it does |
|---|---|
| `just test` | Offline suite: `cargo test --lib --bins --tests`. `#[ignore]` tests stay skipped. No token, no Docker. |
| `just test-embeddings` | Live embeddings + vector memory (`tests/live.rs`). Needs the API token. |
| `just test-live` | The whole ignored `tests/live.rs` suite. Needs the token and Docker. Hits the network. |
| `just lint` | `cargo fmt -- --check` and `cargo clippy --all-targets -- -D warnings` |
| `just fmt` | `cargo fmt` |
| `just clean` | `cargo clean` |
| `just clean-projects` | Deletes `examples/.hal` |
| `just show-config FILE` | Looks for a `show_config` example that is not in the tree yet, then tells you to read `.hal/config.toml` and the environment |

After a Rust change: `just fmt`, then `just lint` and `just test`. Run `just test-embeddings` or `just test-live` when the change touches the live xAI or container path and a token is configured.

`src/container.rs` has ignored Docker tests (`hello_from_container`, `host_cwd_mount_resolves_relative_args`, `interrupt_reaches_sleeping_script`, `cancelled_run_closes_output_and_removes_container`). When you change mount or run behavior:

```sh
cargo test --lib container:: -- --ignored --nocapture
```

`just test` takes no filter. Narrow an offline test with cargo:

```sh
cargo test --lib project::
cargo test --test cli_smoke
```

## Dev setup

- Rust stable, edition 2021. The crate declares 1.75 as the floor. `Cargo.lock` is committed, and release builds pass `--locked`.
- A C toolchain, `pkg-config`, and OpenSSL headers (`libssl-dev`). rig-core and reqwest use `native-tls` so they share the system OpenSSL and avoid compiling `aws-lc-sys`. Keep that feature set.
- `just` on `PATH`. The dev environment already has it.
- Docker, only to execute a `.hal` file or the live and container tests. `just build` and `just test` do not need it.
- `HAL_API_TOKEN` for any model or embedding call.

Token load order at startup (`src/main.rs`), first hit wins:

1. Variables already in the process environment.
2. `.env` discovered from the working directory (`dotenvy::dotenv()`).
3. `HAL/.env`, which fills keys that are still unset.

`.env` is gitignored. Never commit a token, and never print its value.

Model and runtime settings resolve in `Config::load_for_source` (`src/config.rs`), highest priority first:

1. Environment: `HAL_MODEL`, `HAL_EMBEDDING_MODEL`, `HAL_EMBEDDING_DIMS`, `HAL_DEFAULT_IMAGE`, `HAL_BASE_URL`, `HAL_REFLECTOR`, `HAL_REFLECTOR_MAX_TURNS`, `HAL_MEMORY_SEARCH_K`, `HAL_API_TOKEN`.
2. `<source_parent>/.hal/config.toml`. For `examples/fizzbuzz.hal` that file is `examples/.hal/config.toml`, next to the per-project `<id>/` directory.
3. Built-in defaults: model `grok-4.20-0309-reasoning`, embeddings `embedding-beta-3-large` at 4096 dims, image `debian:stable`, reflection on (max 6 turns), memory search k = 5.

The TOML struct uses `deny_unknown_fields`. A missing file is fine. There is no `.devcontainer/` in the tree.

Logs: every invocation appends to `~/.hal/logs/hal.log`. Override the directory with `HAL_LOG_DIR`, otherwise `$XDG_STATE_HOME/hal/logs` when that is set. Stderr logging stays off unless `-v` or `HAL_LOG_STDERR=1`, because it fights the TUI. `RUST_LOG` and `HAL_LOG` use `tracing` `EnvFilter` syntax. The default filter is `hal=debug,bollard=info,rig=info`. Each project also writes `<project>/.hal/<id>/logs/`. `HAL_TRACE_EVENTS=/path/to/trace.jsonl` records the event bus.

The `[profile.release]` in `Cargo.toml` prefers install speed: `lto = false`, `codegen-units = 16`, symbols stripped, and `build-override` opt-level 0 for C build scripts. Leave it unless you are changing build performance on purpose.

## Where things live

| Path | Role |
|---|---|
| `src/main.rs` | clap CLI |
| `src/commands/build.rs`, `run.rs` | `hal build` and `hal run` |
| `src/pipeline.rs` | Interpreter, then Architect; writes `build.lock` |
| `src/agents/interpreter.rs`, `architect.rs` | LLM agents |
| `src/agents/conductor.rs` | Deterministic Docker run. Keep LLM calls off this path. |
| `src/agents/reflector.rs` | LLM agent after the container exits (identity `HAL/reflector.md`); read-only files, `run_cmd` probes; writes the `reflector` memory namespace. The Conductor writes only bare run facts to `conductor`. |
| `src/container.rs` | bollard: create, mount `/work`, stream logs, remove |
| `src/llm.rs` | xAI client and embedder |
| `src/memory.rs` | Per-project SQLite. Cosine search when an embedder exists; otherwise recency plus a `LIKE` filter |
| `src/tools/` | `files`, `memory`, `shell`, `spawn_agent` (global cap 4), `web_search` |
| `src/tui/` | ratatui dashboard. Logos come from `HAL/*.logo` (one per agent; the Reflector is light green) |
| `src/config.rs`, `project.rs`, `logging.rs`, `events.rs`, `control.rs` | config, on-disk layout, logs, event bus, pause |
| `HAL/*.md` | Identity per agent (`interpreter`, `architect`, `conductor`, `reflector`), tool reference, and flow. `reflector.md` is the Reflector system prompt. Embedded with `include_str!` from `src/lib.rs` (`identity`) and `src/tui/theme.rs` |
| `examples/*.hal` | Smoke scripts: `fizzbuzz.hal`, `chron.hal` (with `examples/jobs.txt`), `http_ping.hal`, `test.hal`. Showcase scripts: `aquarium.hal` (terminal animation), `quakes.hal` (web server + live API), `pet.hal` (state across runs), `git-story.hal` (reads/writes the host folder), `synth.hal` (binary .wav output), `vague.hal` (minimal spec) |
| `tests/cli_smoke.rs` | Offline CLI tests, included in `just test` |
| `tests/live.rs` | Ignored live tests, run via `just test-live` |
| `install.sh` | Installer behind `just install` |

Editing `HAL/*.md` or `HAL/*.logo` changes the binary only after a rebuild. Those files are what the model and the TUI actually see. If you change a tool's contract in `src/tools/`, update `HAL/tools.md` (and the identity doc that mentions it) in the same change.

## Generated project tree

`Project::open` hashes the canonical absolute source path with blake3 and keeps the first 16 hex characters. The same file always maps to the same directory, including across `cd`.

```
examples/fizzbuzz.hal
examples/.hal/config.toml       # optional; shared by every source in this directory
examples/.hal/<id>/
  logic.md                      # Interpreter
  artifacts/                    # Architect; mounted at /work (rw unless runtime.yaml sets artifacts_rw: false)
  runtime.yaml                  # image + entrypoint
  build.lock                    # source hash, image, entrypoint, built_at
  memory/hal.sqlite
  runs/<ts>/                    # stdout.log, stderr.log, meta.json
  logs/
```

`.hal/`, `target/`, `*.log`, and `.env` are gitignored. `just clean-projects` removes only `examples/.hal`.

`hal run` skips Interpreter and Architect when `build.lock`'s `source_hash` matches the file and both `logic.md` and `runtime.yaml` are still on disk. `-f` / `--force`, a hash mismatch, or a missing artifact wipes `logic.md`, `runtime.yaml`, `build.lock`, and `artifacts/*`. `memory/` and `runs/` stay.

## CLI

```
hal build <file.hal>            # Interpreter + Architect
hal run <file.hal> [args...]    # build if stale, then Conductor + Reflector
hal <file.hal> [args...]        # shorthand for run
-d / --direct                   # no TUI; container stdio only
-f / --force                    # rebuild even when build.lock matches
-v / --verbose                  # also log to stderr
```

Tokens after the file path are the script's own arguments (the `chron.hal` / `jobs.txt` pattern), and that includes anything that looks like a flag, so put `-d` / `-f` / `-v` before the file. The Architect sees the arguments at build time so `runtime.yaml` can plan for them.

Stopping a run: in `-d` mode the first Ctrl-C sends SIGINT to the container's whole process group (containers run with `--init`), waits up to 10s (`INTERRUPT_GRACE` in `src/container.rs`), then kills it. A second Ctrl-C force-quits. Quitting the TUI mid-run does the same. An interrupted run skips the Reflector. With no container running, Ctrl-C exits at once.

## While you edit

- Match the style of the module you are in. Clippy is `-D warnings` under `just lint`. Fix the warning.
- Keep the Conductor deterministic. Remembering what the run did belongs in the Reflector.
- Offline tests pass with no token and no Docker. Put live xAI coverage in `tests/live.rs` behind `#[ignore]`. Put Docker-only unit coverage next to the code, ignored, the way `src/container.rs` does.
- Leave generated `.hal/` trees, `target/`, and `.env` untracked.
