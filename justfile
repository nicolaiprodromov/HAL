# HAL command runner.
#
# Install `just` (https://just.systems) and run `just` for the list of
# recipes, or `just <recipe>` to execute one. The dev container ships
# with `just` already on PATH.

# Default recipe: show the list.
default:
    @just --list

# -- build / install --------------------------------------------------------
#
# All recipes below share the workspace `target/` directory, so a debug
# build, a release build, and an install all benefit from each other's
# incremental cache. `just install` and `./install.sh` are the SAME thing:
# `install` just delegates to the shell script.

# Debug build (fast; use for day-to-day iteration).
build:
    cargo build --bin hal

# Release build — what `just install` / `./install.sh` ship.
release:
    cargo build --release --locked --bin hal

# Install the `hal` binary into `~/.cargo/bin` (must be on PATH).
# Delegates to install.sh so this and the shell script stay in lockstep.
install:
    ./install.sh

# Install into /usr/local/bin (needs sudo).
install-system:
    ./install.sh --system

# -- run --------------------------------------------------------------------
#
# Dev recipes default to DEBUG builds — they compile ~3–5× faster than
# release and are plenty fast enough to execute .hal files. Use the
# `run-release` variants when you specifically want an optimised binary.

# Example: just run examples/chron.hal examples/jobs.txt
# Run a .hal file with the TUI (debug build). Extra args go to the script.
run FILE *ARGS:
    cargo run --bin hal -- {{FILE}} {{ARGS}}

# Flags must go before FILE: anything after it is passed to the script.
# Run a .hal file detached (-d): only the container output is shown (debug build).
run-direct FILE *ARGS:
    cargo run --bin hal -- -d {{FILE}} {{ARGS}}

# Build (no run) a .hal file, materialising .hal/<id>/artifacts/ (debug build).
build-hal FILE:
    cargo run --bin hal -- build {{FILE}}

# Release-mode variants, for when you actually want the optimised binary.
run-release FILE *ARGS:
    cargo run --release --bin hal -- {{FILE}} {{ARGS}}

run-direct-release FILE *ARGS:
    cargo run --release --bin hal -- -d {{FILE}} {{ARGS}}

# -- tests / quality --------------------------------------------------------

# Fast offline suite.
test:
    cargo test --lib --bins --tests

# Run the live API/Docker suite (needs HAL_API_TOKEN + dockerd).
test-live:
    cargo test --test live -- --ignored --nocapture

# Just the embeddings live check (fastest live test).
test-embeddings:
    cargo test --test live embeddings_and_vector_memory -- --ignored --nocapture

# rustfmt + clippy (-D warnings). CI-equivalent quality gate.
lint:
    cargo fmt -- --check
    cargo clippy --all-targets -- -D warnings

# Apply rustfmt.
fmt:
    cargo fmt

# -- housekeeping -----------------------------------------------------------

# Remove the build cache.
clean:
    cargo clean

# Remove all generated `.hal/` project bundles under examples/.
clean-projects:
    rm -rf examples/.hal
    @echo "wiped examples/.hal"

# Show the resolved config for a .hal source file (env > .hal/config.toml > defaults).
show-config FILE:
    cargo run --quiet --example show_config -- {{FILE}} 2>/dev/null || \
        echo "(no show_config example yet; inspect <dir>/.hal/config.toml and env manually)"
