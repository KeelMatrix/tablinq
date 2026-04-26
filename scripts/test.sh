#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

cd "$ROOT_DIR"

if [[ ! -d node_modules ]]; then
  pnpm install --frozen-lockfile
fi

start_seconds="$(date +%s)"
cargo test --quiet -p gpt --locked
cargo test --quiet -p gptd --locked
cargo test --quiet -p shared-protocol --locked
cargo test --quiet -p tablinq-host --locked

gateway_test_filters=(
  'auth::'
  'errors::'
  'openai::'
  'routes::'
  'tests::capabilities_provider'
  'tests::chat_requests'
  'tests::conversation_inspection'
  'tests::failure_recovery_status'
  'tests::gateway_process'
  'tests::request_parsing'
  'tests::scheduler_process_control'
  'tests::uploads_artifacts'
)

for filter in "${gateway_test_filters[@]}"; do
  cargo test --quiet -p tablinq-gateway --locked "$filter"
done

pnpm exec vitest run --reporter=dot
elapsed_seconds="$(( $(date +%s) - start_seconds ))"
printf 'All local tests passed in %02d:%02d.\n' "$((elapsed_seconds / 60))" "$((elapsed_seconds % 60))"
