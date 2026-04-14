#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

cd "$ROOT_DIR"

cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
pnpm lint
pnpm build
pnpm test
