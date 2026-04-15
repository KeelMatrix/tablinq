#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PROFILE="${1:-release}"
OUTPUT_ROOT="${OUTPUT_ROOT:-$ROOT_DIR/dist/package/tablinq}"
TARGET_ROOT="${CARGO_TARGET_DIR:-$ROOT_DIR/target}"

abspath() {
  local path="$1"
  if [[ "$path" == /* ]]; then
    printf '%s\n' "$path"
  else
    printf '%s\n' "$ROOT_DIR/$path"
  fi
}

OUTPUT_ROOT="$(abspath "$OUTPUT_ROOT")"
TARGET_ROOT="$(abspath "$TARGET_ROOT")"

resolve_target_profile_dir() {
  case "$1" in
    dev) printf '%s\n' "debug" ;;
    *) printf '%s\n' "$1" ;;
  esac
}

mkdir -p "$OUTPUT_ROOT/bin"
rm -rf "$OUTPUT_ROOT"
mkdir -p "$OUTPUT_ROOT/bin"

(
  cd "$ROOT_DIR"
  cargo_args=(build --profile "$PROFILE" -p gpt -p gptd -p tablinq-gateway -p tablinq-host)
  if [[ -n "${CARGO_TARGET_DIR:-}" ]]; then
    cargo_args+=(--target-dir "$TARGET_ROOT")
  fi
  cargo "${cargo_args[@]}"
  pnpm --filter @tablinq/browser-adapter build
  pnpm --filter @tablinq/extension build
  pnpm --filter @tablinq/browser-adapter deploy --legacy --prod "$OUTPUT_ROOT/browser-adapter"
)

TARGET_DIR="$TARGET_ROOT/$(resolve_target_profile_dir "$PROFILE")"
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
cp "$ROOT_DIR/LICENSE" "$OUTPUT_ROOT/LICENSE"
cp "$ROOT_DIR/README.md" "$OUTPUT_ROOT/README.md"

extension_id="$(python - <<'PY' "$OUTPUT_ROOT/extension/manifest.json"
import base64
import hashlib
import json
import sys

with open(sys.argv[1], "r", encoding="utf-8") as fh:
    manifest = json.load(fh)

key = manifest.get("key")
if not key:
    raise SystemExit("Extension manifest is missing a stable key")

digest = hashlib.sha256(base64.b64decode(key)).digest()
alphabet = "abcdefghijklmnop"
chars = []
for byte in digest[:16]:
    chars.append(alphabet[(byte >> 4) & 0x0F])
    chars.append(alphabet[byte & 0x0F])
print("".join(chars))
PY
)"

"$OUTPUT_ROOT/bin/gpt" version --json > "$OUTPUT_ROOT/.package-version.json"
python - <<'PY' "$OUTPUT_ROOT/.package-version.json" "$OUTPUT_ROOT/package-metadata.json" "$OUTPUT_ROOT" "$extension_id"
import json
import os
import sys

version_path, metadata_path, output_root, extension_id = sys.argv[1:5]
with open(version_path, "r", encoding="utf-8") as fh:
    build = json.load(fh)

bin_root = os.path.join(output_root, "bin")
metadata = {
    "package_version": build["version"],
    "commit": build["git_commit"],
    "dirty": build["git_dirty"],
    "target": build["target"],
    "profile": build["profile"],
    "built_at_unix_epoch": build["built_at_unix_epoch"],
    "extension_id": extension_id,
    "cli_binary": os.path.join(bin_root, "gpt"),
    "daemon_binary": os.path.join(bin_root, "gptd"),
    "gateway_binary": os.path.join(bin_root, "tablinq-gateway"),
    "native_host_binary": os.path.join(bin_root, "tablinq-host"),
    "extension_path": os.path.join(output_root, "extension"),
    "browser_adapter_entry": os.path.join(output_root, "browser-adapter", "src", "service.js"),
}

with open(metadata_path, "w", encoding="utf-8") as fh:
    json.dump(metadata, fh, indent=2)
PY
rm -f "$OUTPUT_ROOT/.package-version.json"
