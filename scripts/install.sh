#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PACKAGE_ROOT="${PACKAGE_ROOT:-$ROOT_DIR/dist/package/tablinq}"
INSTALL_ROOT="${INSTALL_ROOT:-${XDG_DATA_HOME:-$HOME/.local/share}/tablinq/install}"
STATE_ROOT="${STATE_ROOT:-${XDG_STATE_HOME:-$HOME/.local/state}/tablinq}"

rm -rf "$INSTALL_ROOT"
mkdir -p "$INSTALL_ROOT"
cp -R "$PACKAGE_ROOT/." "$INSTALL_ROOT/"
mkdir -p "$STATE_ROOT"

"$INSTALL_ROOT/bin/gpt" --state-root "$STATE_ROOT" config show >/dev/null

echo "Installed Tablinq to $INSTALL_ROOT"
echo "State root: $STATE_ROOT"
echo "Load the unpacked extension from $INSTALL_ROOT/extension"
echo "Then run:"
echo "  $INSTALL_ROOT/bin/gpt daemon start"
echo "  $INSTALL_ROOT/bin/gpt browser open"
