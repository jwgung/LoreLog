#!/usr/bin/env bash
# What a change to the server library must pass before it is committed: tests, lint, format, and a Windows build
# check — the same library runs inside the desktop app (docs/adr/0002), so it must keep compiling for Windows.
# Builds nothing in release, so it never touches binaries a running service might use.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"

cargo test -q -p life-server
cargo clippy -q -p life-server --all-targets -- -D warnings
cargo fmt --check -p life-server

if cargo xwin --version >/dev/null 2>&1; then
    cargo xwin check -q --target x86_64-pc-windows-msvc -p life-server
    echo "windows: ok"
else
    echo "windows: FAILED, cargo-xwin is not installed (cargo install cargo-xwin)" >&2
    exit 1
fi
echo "all checks passed"
