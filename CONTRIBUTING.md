# Contributing to HAL

Thanks for your interest in HAL. It is a research project and development
is slow, so expect reviews to take a while. Bug reports and small, focused
pull requests are the easiest to land.

## Before you open a pull request

1. `just fmt`
2. `just lint` (rustfmt check + clippy with `-D warnings`)
3. `just test` (offline: needs no token and no Docker)

If your change touches the live model API or container path and you have a token,
also run `just test-embeddings` or `just test-live`. Keep the Conductor
deterministic: anything that needs an LLM belongs in the Reflector. Never
commit a `.env` file or an API token.

## Development setup

Install [`just`](https://just.systems) and run `just` to list recipes.
Every recipe shares this repo's `target/` directory. Day-to-day recipes
compile the debug binary, which is enough to execute `.hal` files. Use
the release recipes when you want the optimized binary that `just install`
ships.

```sh
just build                 # cargo build --bin hal
just release               # cargo build --release --locked --bin hal
just install               # ./install.sh → ~/.cargo/bin
just install-system        # ./install.sh --system

just run examples/fizzbuzz.hal            # debug, TUI
just run-direct examples/fizzbuzz.hal     # debug, detached (-d)
just build-hal examples/fizzbuzz.hal      # debug `hal build`, no container
just run-release FILE                     # release, TUI
just run-direct-release FILE              # release, detached

just fmt                   # cargo fmt
just lint                  # rustfmt --check + clippy -D warnings
just clean                 # cargo clean
just clean-projects        # wipe examples/.hal
just show-config FILE      # resolved config for a source file
```

`just show-config` tries a `show_config` example that is not in the tree
yet, then prints a hint to inspect `<dir>/.hal/config.toml` and the
environment.

A checkout needs Rust, a C toolchain, OpenSSL headers, and `just`. Docker
is required to run a `.hal` file. This repo does not ship a
`.devcontainer/`. Put `HAL_API_TOKEN` in the environment or in `.env`
/ `HAL/.env` before `just build-hal`, `just run`, or the live tests.

Logs for every invocation append to `~/.hal/logs/hal.log`, or to
`$XDG_STATE_HOME/hal/logs` when that variable is set. `HAL_LOG_DIR`
overrides both. `-v` also prints them on stderr. `RUST_LOG` and `HAL_LOG`
use `tracing` `EnvFilter` syntax. The default filter is
`hal=debug,bollard=info,rig=info`.

The release profile in `Cargo.toml` is tuned for install speed (`lto` off,
`codegen-units = 16`, symbols stripped). Identity docs in [`HAL/`](./HAL/)
are compiled in with `include_str!`, so a prompt or logo edit shows up
only after a rebuild.

## Testing

```sh
just test               # offline suite: cargo test --lib --bins --tests
just test-embeddings    # live embeddings + vector memory (needs the token)
just test-live          # tests/live.rs, ignored tests (token + Docker)
```

`just test` skips `#[ignore]` tests and needs neither a token nor Docker.
`just test-live` runs the live API suite, including a container run.

The ignored Docker tests in `src/container.rs` are separate from
`just test-live`:

```sh
cargo test --lib container:: -- --ignored --nocapture
```
