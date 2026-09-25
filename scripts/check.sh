#!/usr/bin/env bash
# Local CI: the same checks as .github/workflows/ci.yml, in the same order.
set -euo pipefail
cd "$(dirname "$0")/.."
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"

echo "== fmt";    cargo fmt --all -- --check
echo "== clippy"; cargo clippy --workspace --all-targets -- -D warnings
echo "== test";   cargo test --workspace
echo "== wasm";   stellar contract build
ls -l target/wasm32v1-none/release/*.wasm
echo "== all green"
