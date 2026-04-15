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
mkdir -p "$INSTALL_ROOT/startup"

"$INSTALL_ROOT/bin/gpt" --state-root "$STATE_ROOT" config show >/dev/null

cat >"$INSTALL_ROOT/startup/start-tablinq-daemon.sh" <<EOF
#!/usr/bin/env bash
exec "$INSTALL_ROOT/bin/gpt" --state-root "$STATE_ROOT" daemon start
EOF
chmod +x "$INSTALL_ROOT/startup/start-tablinq-daemon.sh"

cat >"$INSTALL_ROOT/startup/start-tablinq-gateway.sh" <<EOF
#!/usr/bin/env bash
exec "$INSTALL_ROOT/bin/gpt" --state-root "$STATE_ROOT" gateway start
EOF
chmod +x "$INSTALL_ROOT/startup/start-tablinq-gateway.sh"

echo "Installed Tablinq to $INSTALL_ROOT"
echo "State root: $STATE_ROOT"
echo "Load the unpacked extension from $INSTALL_ROOT/extension"
echo "Manual steps:"
echo "  1. Load the unpacked extension from $INSTALL_ROOT/extension"
echo "  2. Fully close Chrome or Edge"
echo "  3. Run: $INSTALL_ROOT/bin/gpt daemon start"
echo "  4. Run: $INSTALL_ROOT/bin/gpt gateway start"
echo "  5. Run: $INSTALL_ROOT/bin/gpt browser start-debug"
echo "  6. Sign in manually at https://chatgpt.com/"
echo "  7. Run: $INSTALL_ROOT/bin/gpt browser attach"
echo "  8. Verify: $INSTALL_ROOT/bin/gpt doctor"
echo "Startup helpers:"
echo "  $INSTALL_ROOT/startup/start-tablinq-daemon.sh"
echo "  $INSTALL_ROOT/startup/start-tablinq-gateway.sh"
