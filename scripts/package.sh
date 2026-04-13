#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PROFILE="${1:-release}"
OUTPUT_ROOT="${OUTPUT_ROOT:-$ROOT_DIR/dist/package/tablinq}"

mkdir -p "$OUTPUT_ROOT/bin"
rm -rf "$OUTPUT_ROOT"
mkdir -p "$OUTPUT_ROOT/bin"

(
  cd "$ROOT_DIR"
  cargo build --profile "$PROFILE" -p gpt -p gptd -p tablinq-gateway -p tablinq-host
  pnpm build
  pnpm --filter @tablinq/browser-adapter deploy --legacy --prod "$OUTPUT_ROOT/browser-adapter"
)

TARGET_DIR="$ROOT_DIR/target/$PROFILE"
cp "$TARGET_DIR/gpt" "$OUTPUT_ROOT/bin/gpt"
cp "$TARGET_DIR/gptd" "$OUTPUT_ROOT/bin/gptd"
cp "$TARGET_DIR/tablinq-gateway" "$OUTPUT_ROOT/bin/tablinq-gateway"
cp "$TARGET_DIR/tablinq-host" "$OUTPUT_ROOT/bin/tablinq-host"
mkdir -p "$OUTPUT_ROOT/browser-adapter/src" "$OUTPUT_ROOT/extension/src"
mkdir -p "$OUTPUT_ROOT/browser-adapter/src"
cp -R "$ROOT_DIR/packages/browser-adapter/dist/src/." "$OUTPUT_ROOT/browser-adapter/src/"
cp -R "$ROOT_DIR/packages/extension/dist/src/." "$OUTPUT_ROOT/extension/src/"
cp "$ROOT_DIR/packages/extension/manifest.json" "$OUTPUT_ROOT/extension/manifest.json"
cp -R "$ROOT_DIR/selector-packs" "$OUTPUT_ROOT/"
cp -R "$ROOT_DIR/schemas" "$OUTPUT_ROOT/"
cp -R "$ROOT_DIR/docs" "$OUTPUT_ROOT/"
cp "$ROOT_DIR/README.md" "$OUTPUT_ROOT/README.md"
