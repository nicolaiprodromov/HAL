#!/usr/bin/env bash
# HAL installer — builds `hal` in release mode and drops it on your PATH.
#
# Usage:
#   ./install.sh              # installs to ~/.cargo/bin/hal
#   ./install.sh --system     # installs to /usr/local/bin/hal (needs sudo)
#
# Notes on build speed:
#   This script intentionally avoids `cargo install --path .`. That command
#   compiles in a throwaway target directory, so every invocation is a
#   full clean rebuild from scratch — which for HAL means re-compiling large
#   native deps (sqlite, blake3, OpenSSL bindings, etc.) every time.
#
#   Instead we run `cargo build --release` inside the workspace `target/`
#   directory (so incremental compilation actually works) and then copy
#   the produced binary into place. Subsequent installs after a small
#   code change finish in seconds rather than minutes.
#
# Requires: Rust 1.75+ (stable), a C toolchain, pkg-config, libssl-dev.
# `curl https://sh.rustup.rs -sSf | sh` if you don't have rustc yet.

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$HERE"

MODE="${1:-user}"

if ! command -v cargo >/dev/null 2>&1; then
    echo "error: cargo not found on PATH. Install Rust first:" >&2
    echo "  curl https://sh.rustup.rs -sSf | sh" >&2
    exit 1
fi

# Use all available cores but leave one free so the machine stays responsive
# during the build. Override with CARGO_BUILD_JOBS=N ./install.sh if desired.
if [[ -z "${CARGO_BUILD_JOBS:-}" ]]; then
    if command -v nproc >/dev/null 2>&1; then
        jobs="$(nproc)"
    else
        jobs=4
    fi
    if (( jobs > 2 )); then
        jobs=$(( jobs - 1 ))
    fi
    export CARGO_BUILD_JOBS="$jobs"
fi

build_release() {
    echo "==> cargo build --release --locked  (jobs=${CARGO_BUILD_JOBS})"
    cargo build --release --locked --bin hal
}

install_user() {
    build_release
    local dest="${CARGO_HOME:-$HOME/.cargo}/bin"
    mkdir -p "$dest"
    install -m 0755 target/release/hal "$dest/hal"
    echo
    if ! command -v hal >/dev/null 2>&1; then
        echo "'hal' was installed to $dest but that directory is not on your PATH."
        echo "Add this to your shell profile:"
        echo "  export PATH=\"$dest:\$PATH\""
    else
        echo "installed: $(command -v hal)"
        hal --version || true
    fi
}

install_system() {
    build_release
    echo "==> sudo install target/release/hal /usr/local/bin/hal"
    sudo install -m 0755 target/release/hal /usr/local/bin/hal
    echo
    echo "installed: $(command -v hal)"
    hal --version || true
}

case "$MODE" in
    user|-u|--user|"")
        install_user
        ;;
    --system|-s|system)
        install_system
        ;;
    *)
        echo "usage: $0 [--user|--system]" >&2
        exit 2
        ;;
esac
