#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PACKAGE_ROOT="${PACKAGE_ROOT:-$ROOT_DIR/dist/package/tablinq}"
INSTALL_ROOT="${INSTALL_ROOT:-${XDG_DATA_HOME:-$HOME/.local/share}/tablinq/install}"
STATE_ROOT="${STATE_ROOT:-${XDG_STATE_HOME:-$HOME/.local/state}/tablinq}"

GPT_BIN_NAME="gpt"
GPT_BIN="$INSTALL_ROOT/bin/$GPT_BIN_NAME"
DAEMON_BIN="$INSTALL_ROOT/bin/gptd"
GATEWAY_BIN="$INSTALL_ROOT/bin/tablinq-gateway"
HOST_BIN="$INSTALL_ROOT/bin/tablinq-host"
EXTENSION_PATH="$INSTALL_ROOT/extension"
CHROME_ATTACH_PROFILE_DIR="$STATE_ROOT/chrome-attach-profile"
CONFIG_DIR="$STATE_ROOT/config"
CONFIG_PATH="$CONFIG_DIR/config.toml"

browser_name=""
browser_channel=""
browser_path=""

fail() {
  printf '%s\n' "$*" >&2
  exit 1
}

require_command() {
  local command_name="$1"
  local install_hint="$2"
  if ! command -v "$command_name" >/dev/null 2>&1; then
    fail "Missing required command '$command_name'. $install_hint"
  fi
}

require_path() {
  local path="$1"
  local label="$2"
  if [[ ! -e "$path" ]]; then
    fail "Missing $label: $path"
  fi
}

remove_path_if_exists() {
  local path="$1"
  if [[ -e "$path" ]]; then
    rm -rf "$path"
  fi
}

assert_dir_writable() {
  local path="$1"
  local label="$2"
  local probe_path

  [[ -n "$path" ]] || fail "Missing required path for $label."
  mkdir -p "$path" || fail "$label is not writable at $path. Grant write access or choose a different install location."
  probe_path="$path/.tablinq-write-test-$$-$RANDOM.tmp"
  if ! : >"$probe_path" 2>/dev/null; then
    fail "$label is not writable at $path. Grant write access or choose a different install location."
  fi
  rm -f "$probe_path"
}

resolve_browser_selection() {
  local candidate_name candidate_channel candidate_path command_name resolved
  local -a candidates=(
    "Chrome|chrome|google-chrome"
    "Chrome|chrome|google-chrome-stable"
    "Chrome|chrome|chromium"
    "Chrome|chrome|chromium-browser"
    "Edge|msedge|microsoft-edge"
    "Edge|msedge|microsoft-edge-stable"
    "Chrome|chrome|/usr/bin/google-chrome"
    "Chrome|chrome|/usr/bin/google-chrome-stable"
    "Chrome|chrome|/opt/google/chrome/chrome"
    "Chrome|chrome|/usr/bin/chromium"
    "Chrome|chrome|/usr/bin/chromium-browser"
    "Edge|msedge|/usr/bin/microsoft-edge"
    "Edge|msedge|/usr/bin/microsoft-edge-stable"
    "Edge|msedge|/opt/microsoft/msedge/msedge"
  )

  for candidate in "${candidates[@]}"; do
    IFS="|" read -r candidate_name candidate_channel candidate_path <<<"$candidate"
    if [[ "$candidate_path" == */* ]]; then
      if [[ -x "$candidate_path" ]]; then
        browser_name="$candidate_name"
        browser_channel="$candidate_channel"
        browser_path="$candidate_path"
        return 0
      fi
      continue
    fi

    if resolved="$(command -v "$candidate_path" 2>/dev/null)"; then
      browser_name="$candidate_name"
      browser_channel="$candidate_channel"
      browser_path="$resolved"
      return 0
    fi
  done

  for command_name in google-chrome google-chrome-stable chromium chromium-browser microsoft-edge microsoft-edge-stable; do
    if resolved="$(command -v "$command_name" 2>/dev/null)"; then
      case "$command_name" in
        microsoft-edge|microsoft-edge-stable)
          browser_name="Edge"
          browser_channel="msedge"
          ;;
        *)
          browser_name="Chrome"
          browser_channel="chrome"
          ;;
      esac
      browser_path="$resolved"
      return 0
    fi
  done

  return 1
}

set_toml_string_value() {
  local path="$1"
  local key="$2"
  local value="$3"
  local escaped_value line tmp_file

  escaped_value="${value//\'/\'\'}"
  line="$key = '$escaped_value'"
  tmp_file="$(mktemp "${TMPDIR:-/tmp}/tablinq-config.XXXXXX")"

  if [[ -f "$path" ]] && grep -Eq "^${key}[[:space:]]*=" "$path"; then
    awk -v key="$key" -v line="$line" '
      BEGIN { replaced = 0 }
      $0 ~ ("^" key "[[:space:]]*=") && replaced == 0 {
        print line
        replaced = 1
        next
      }
      { print }
    ' "$path" >"$tmp_file"
  else
    if [[ -f "$path" ]]; then
      cat "$path" >"$tmp_file"
      if [[ -s "$tmp_file" ]]; then
        printf '\n' >>"$tmp_file"
      fi
    fi
    printf '%s\n' "$line" >>"$tmp_file"
  fi

  mv "$tmp_file" "$path"
}

get_toml_string_value() {
  local path="$1"
  local key="$2"
  local default_value="$3"
  local value

  if [[ ! -f "$path" ]]; then
    printf '%s\n' "$default_value"
    return 0
  fi

  value="$(
    awk -v key="$key" '
      match($0, "^" key "[[:space:]]*=[[:space:]]*[\047\042](.*)[\047\042][[:space:]]*$", parts) {
        print parts[1]
        exit
      }
    ' "$path"
  )"

  if [[ -n "$value" ]]; then
    printf '%s\n' "$value"
  else
    printf '%s\n' "$default_value"
  fi
}

ensure_config_file() {
  mkdir -p "$CONFIG_DIR"
  if [[ ! -f "$CONFIG_PATH" ]]; then
    cat >"$CONFIG_PATH" <<'EOF'
version = 1
warm_mode = false
safe_mode = false
browser_target_url = 'https://chatgpt.com/'

[gateway]
host = '127.0.0.1'
port = 3847
EOF
  fi
}

stop_process_by_executable() {
  local executable_path="$1"
  local pid args

  [[ -x "$executable_path" ]] || return 0

  while IFS= read -r line; do
    [[ -n "$line" ]] || continue
    pid="${line%% *}"
    args="${line#* }"
    [[ "$args" == *"$executable_path"* ]] || continue
    kill "$pid" 2>/dev/null || true
  done < <(ps -eo pid=,args= 2>/dev/null || true)
}

stop_browser_processes_for_state_root() {
  local state_root="$1"
  local profile_dir="$state_root/browser-profile"
  local pid args

  while IFS= read -r line; do
    [[ -n "$line" ]] || continue
    pid="${line%% *}"
    args="${line#* }"
    case "$args" in
      *chrome*|*chromium*|*microsoft-edge*|*msedge*)
        if [[ "$args" == *"$profile_dir"* || "$args" == *"$state_root"* ]]; then
          kill "$pid" 2>/dev/null || true
        fi
        ;;
    esac
  done < <(ps -eo pid=,args= 2>/dev/null || true)
}

write_startup_script() {
  local script_path="$1"
  shift
  cat >"$script_path" <<EOF
#!/usr/bin/env bash
set -euo pipefail
exec "$GPT_BIN" --state-root "$STATE_ROOT" "$@"
EOF
  chmod +x "$script_path"
}

print_manual_steps() {
  local daemon_running="$1"
  local gateway_running="$2"
  local connection_mode="$3"
  local target_url="$4"
  local step=1

  echo
  echo "Tablinq install is ready."
  echo "Install root: $INSTALL_ROOT"
  echo "State root: $STATE_ROOT"
  echo "Browser: $browser_name"
  echo "Connection mode: $connection_mode"
  echo "Daemon running: $daemon_running"
  echo "Gateway running: $gateway_running"
  if [[ "$connection_mode" == "attach_first" && "$browser_name" == "Chrome" ]]; then
    echo "Chrome attach profile: $CHROME_ATTACH_PROFILE_DIR"
  fi
  echo
  echo "Manual steps:"

  if [[ "$daemon_running" != "true" ]]; then
    echo "$step. Start the daemon: $GPT_BIN --state-root $STATE_ROOT daemon start"
    step=$((step + 1))
  fi

  if [[ "$gateway_running" != "true" ]]; then
    echo "$step. Start the gateway: $GPT_BIN --state-root $STATE_ROOT gateway start"
    step=$((step + 1))
  fi

  case "$connection_mode" in
    managed_only|managed_fallback)
      echo "$step. Run: $GPT_BIN --state-root $STATE_ROOT browser open"
      step=$((step + 1))
      echo "$step. Sign in manually at $target_url in the managed browser window if prompted."
      step=$((step + 1))
      ;;
    *)
      if [[ "$browser_name" == "Chrome" ]]; then
        echo "$step. Fully close Chrome."
        step=$((step + 1))
        echo "$step. Run: $GPT_BIN --state-root $STATE_ROOT browser start-debug"
        step=$((step + 1))
        echo "$step. In the Chrome window opened by that command, open chrome://extensions/, enable Developer mode, remove any previously loaded Tablinq extension in that attach profile, and load the unpacked extension from: $EXTENSION_PATH"
        step=$((step + 1))
        echo "$step. Sign in manually at $target_url in that same dedicated Chrome attach profile."
        step=$((step + 1))
      else
        echo "$step. Open edge://extensions/ in the Edge profile you will attach to, enable Developer mode, remove any previously loaded Tablinq extension, and load the unpacked extension from: $EXTENSION_PATH"
        step=$((step + 1))
        echo "$step. Fully close Edge."
        step=$((step + 1))
        echo "$step. Run: $GPT_BIN --state-root $STATE_ROOT browser start-debug"
        step=$((step + 1))
        echo "$step. Sign in manually at $target_url in that same Edge profile."
        step=$((step + 1))
      fi
      echo "$step. Run: $GPT_BIN --state-root $STATE_ROOT browser attach"
      step=$((step + 1))
      ;;
  esac

  echo "$step. Verify browser session: $GPT_BIN --state-root $STATE_ROOT browser connection-info"
  step=$((step + 1))
  echo "$step. Verify overall status: $GPT_BIN --state-root $STATE_ROOT status"
  step=$((step + 1))
  echo "$step. Verify diagnostics: $GPT_BIN --state-root $STATE_ROOT doctor"
  step=$((step + 1))
  echo "$step. Inspect capabilities: $GPT_BIN --state-root $STATE_ROOT capabilities"
  step=$((step + 1))
  echo "$step. Try a smoke prompt: $GPT_BIN --state-root $STATE_ROOT ask \"hello\""

  if [[ "$connection_mode" == "managed_only" || "$connection_mode" == "managed_fallback" ]]; then
    echo
    echo "Managed browser mode is a fallback path. Use attach-first unless your machine or browser profile cannot expose a usable local debug endpoint."
  fi
}

echo "Preparing Tablinq Unix install from package layout..."

require_command bash "A POSIX-like shell environment with bash is required."
require_command cp "Core file-copy utilities are required."
require_command mkdir "Core directory-creation utilities are required."
require_command rm "Core file-removal utilities are required."
require_command ps "Process inspection is required to stop prior managed processes cleanly."
require_command mktemp "Temporary-file support is required."

require_path "$PACKAGE_ROOT" "package root"
resolve_browser_selection || fail "Missing required browser. Install Google Chrome, Chromium, or Microsoft Edge and make sure the browser executable is discoverable on this system."

assert_dir_writable "$(dirname "$INSTALL_ROOT")" "Install root parent"
assert_dir_writable "$(dirname "$STATE_ROOT")" "State root parent"
assert_dir_writable "$INSTALL_ROOT" "Install root"
assert_dir_writable "$STATE_ROOT" "State root"

existing_bin_dir="$INSTALL_ROOT/bin"
stop_process_by_executable "$existing_bin_dir/gptd"
stop_process_by_executable "$existing_bin_dir/tablinq-gateway"
stop_browser_processes_for_state_root "$STATE_ROOT"
sleep 1

staging_root="${INSTALL_ROOT}.new"
backup_root="${INSTALL_ROOT}.old"
remove_path_if_exists "$staging_root"
remove_path_if_exists "$backup_root"

cleanup() {
  remove_path_if_exists "$staging_root"
}
trap cleanup EXIT

mkdir -p "$staging_root"
cp -R "$PACKAGE_ROOT/." "$staging_root/"

mkdir -p \
  "$STATE_ROOT" \
  "$STATE_ROOT/config" \
  "$STATE_ROOT/logs" \
  "$STATE_ROOT/db" \
  "$STATE_ROOT/browser-profile" \
  "$STATE_ROOT/chrome-attach-profile" \
  "$STATE_ROOT/downloads" \
  "$STATE_ROOT/crash-bundles" \
  "$STATE_ROOT/gateway-uploads" \
  "$staging_root/startup"

chmod +x \
  "$staging_root/bin/gpt" \
  "$staging_root/bin/gptd" \
  "$staging_root/bin/tablinq-gateway" \
  "$staging_root/bin/tablinq-host"

if [[ -e "$INSTALL_ROOT" ]]; then
  mv "$INSTALL_ROOT" "$backup_root"
fi
mv "$staging_root" "$INSTALL_ROOT"
remove_path_if_exists "$backup_root"

require_path "$GPT_BIN" "installed CLI binary"
require_path "$DAEMON_BIN" "installed daemon binary"
require_path "$GATEWAY_BIN" "installed gateway binary"
require_path "$HOST_BIN" "installed native host binary"
require_path "$EXTENSION_PATH" "installed extension directory"

ensure_config_file
set_toml_string_value "$CONFIG_PATH" "browser_path" "$browser_path"
set_toml_string_value "$CONFIG_PATH" "browser_channel" "$browser_channel"

connection_mode="$(get_toml_string_value "$CONFIG_PATH" "browser_connection_mode" "attach_first")"
target_url="$(get_toml_string_value "$CONFIG_PATH" "browser_target_url" "https://chatgpt.com/")"

"$GPT_BIN" --state-root "$STATE_ROOT" config show >/dev/null

write_startup_script "$INSTALL_ROOT/startup/start-tablinq-daemon.sh" daemon start
write_startup_script "$INSTALL_ROOT/startup/start-tablinq-gateway.sh" gateway start

daemon_running="false"
gateway_running="false"

if "$GPT_BIN" --state-root "$STATE_ROOT" daemon start >/dev/null 2>&1; then
  daemon_running="true"
fi

if "$GPT_BIN" --state-root "$STATE_ROOT" gateway start >/dev/null 2>&1; then
  gateway_running="true"
fi

trap - EXIT
print_manual_steps "$daemon_running" "$gateway_running" "$connection_mode" "$target_url"
