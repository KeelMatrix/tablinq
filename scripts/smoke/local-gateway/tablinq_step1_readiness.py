#!/usr/bin/env python3
"""
Tablinq gateway step 1 readiness smoke, v2.

Scope:
- No chat/completions requests.
- No prompt submission.
- No deep research.
- Logs raw CLI I/O, raw gateway HTTP headers/body, and redacted cURL equivalents.
- Prints progress to the PowerShell console.
- Always writes a final verdict block to the markdown log.
"""

from __future__ import annotations

import argparse
import datetime as _dt
import json
import os
import subprocess
import sys
import time
import traceback
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any, Dict, Iterable, List, Optional, Sequence, Tuple


API_KEY_ENV_NAMES = (
    "TABLINQ_GATEWAY_API_KEY",
    "TABLINQ_API_KEY",
    "GATEWAY_API_KEY",
    "OPENAI_API_KEY",
    "API_KEY",
)

BASE_URL_ENV_NAMES = (
    "TABLINQ_GATEWAY_BASE_URL",
    "GATEWAY_BASE_URL",
    "OPENAI_BASE_URL",
    "BASE_URL",
)

DEBUG_URL_ENV_NAMES = (
    "TABLINQ_BROWSER_DEBUG_ENDPOINT",
    "BROWSER_DEBUG_ENDPOINT",
    "CHROME_DEBUG_ENDPOINT",
)

DEFAULT_ROOT_BASE_URL = "http://127.0.0.1:3847"
DEFAULT_DEBUG_URL = "http://127.0.0.1:9222"


def utc_now_iso() -> str:
    return _dt.datetime.now(_dt.timezone.utc).isoformat(timespec="seconds")


def local_now_for_filename() -> str:
    return _dt.datetime.now().strftime("%Y%m%d-%H%M%S")


def progress(phase: str, message: str) -> None:
    print(f"[{_dt.datetime.now().strftime('%H:%M:%S')}] {phase}: {message}", flush=True)


def parse_env_file(path: Path) -> Dict[str, str]:
    values: Dict[str, str] = {}
    if not path.exists():
        return values

    try:
        lines = path.read_text(encoding="utf-8-sig").splitlines()
    except UnicodeDecodeError:
        lines = path.read_text(encoding="utf-8", errors="replace").splitlines()

    for raw_line in lines:
        line = raw_line.strip()
        if not line or line.startswith("#"):
            continue
        if line.lower().startswith("export "):
            line = line[7:].strip()
        if "=" not in line:
            continue

        key, value = line.split("=", 1)
        key = key.strip()
        value = value.strip()
        if not key:
            continue

        if len(value) >= 2:
            if (value[0] == '"' and value[-1] == '"') or (value[0] == "'" and value[-1] == "'"):
                value = value[1:-1]

        values[key] = value

    return values


def first_present_env(names: Iterable[str], env_values: Dict[str, str]) -> Tuple[Optional[str], Optional[str]]:
    for name in names:
        value = os.environ.get(name)
        if value is not None and value.strip():
            return name, value.strip()
    for name in names:
        value = env_values.get(name)
        if value is not None and value.strip():
            return name, value.strip()
    return None, None


def normalize_root_base_url(base_url: str) -> str:
    base_url = base_url.strip().rstrip("/")
    if base_url.endswith("/v1"):
        base_url = base_url[:-3].rstrip("/")
    return base_url or DEFAULT_ROOT_BASE_URL


def normalize_debug_url(debug_url: str) -> str:
    return debug_url.strip().rstrip("/") or DEFAULT_DEBUG_URL


def append_log(log_path: Path, text: str) -> None:
    log_path.parent.mkdir(parents=True, exist_ok=True)
    with log_path.open("a", encoding="utf-8", newline="\n") as f:
        f.write(text)


def fence(label: str, body: Any) -> str:
    text = "" if body is None else str(body)
    text = text.replace("\r\n", "\n").replace("\r", "\n")
    if text and not text.endswith("\n"):
        text += "\n"
    return f"~~~{label}\n{text}~~~\n"


def redact_api_key(text: str, api_key: Optional[str]) -> str:
    if not api_key:
        return text
    return text.replace(api_key, "<REDACTED_API_KEY>")


def quote_for_curl(value: str) -> str:
    return '"' + value.replace("\\", "\\\\").replace('"', '\\"') + '"'


def build_curl(method: str, url: str, headers: Dict[str, str], body_bytes: Optional[bytes]) -> str:
    parts = ["curl.exe", "-i", "-sS", "-X", method.upper(), quote_for_curl(url)]
    for key, value in headers.items():
        parts.extend(["-H", quote_for_curl(f"{key}: {value}")])
    if body_bytes is not None:
        body_text = body_bytes.decode("utf-8", errors="replace")
        parts.extend(["--data-raw", quote_for_curl(body_text)])
    return " ".join(parts)


def response_headers_to_text(status_code: int, reason: str, headers_items: Iterable[Tuple[str, str]]) -> str:
    lines = [f"HTTP/1.1 {status_code} {reason}".rstrip()]
    for key, value in headers_items:
        lines.append(f"{key}: {value}")
    return "\n".join(lines) + "\n"


def try_json(text: str) -> Optional[Any]:
    try:
        return json.loads(text)
    except Exception:
        return None


def find_first_key(value: Any, key: str) -> Any:
    if isinstance(value, dict):
        if key in value:
            return value[key]
        for child in value.values():
            found = find_first_key(child, key)
            if found is not None:
                return found
    elif isinstance(value, list):
        for child in value:
            found = find_first_key(child, key)
            if found is not None:
                return found
    return None


def run_command_capture(
    index: int,
    log_path: Path,
    command: Sequence[str],
    timeout_seconds: int,
    api_key: Optional[str],
    note: str,
) -> Dict[str, Any]:
    rendered = " ".join(command)
    started = utc_now_iso()

    progress("CLI", f"{index:03d} start: {rendered}")

    block_start = []
    block_start.append(f"\n## CLI {index:03d}: `{redact_api_key(rendered, api_key)}`\n\n")
    block_start.append(f"Note: {note}\n\n")
    block_start.append(f"Started UTC: `{started}`  \n")
    append_log(log_path, "".join(block_start))

    stdout = ""
    stderr = ""
    exit_code = 999
    timed_out = False

    try:
        completed = subprocess.run(
            list(command),
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=timeout_seconds,
            shell=False,
        )
        stdout = completed.stdout or ""
        stderr = completed.stderr or ""
        exit_code = int(completed.returncode)
    except FileNotFoundError as exc:
        stderr = f"{type(exc).__name__}: {exc}"
        exit_code = 127
    except subprocess.TimeoutExpired as exc:
        timed_out = True
        stdout = exc.stdout or ""
        stderr = exc.stderr or ""
        if isinstance(stdout, bytes):
            stdout = stdout.decode("utf-8", errors="replace")
        if isinstance(stderr, bytes):
            stderr = stderr.decode("utf-8", errors="replace")
        stderr += f"\nTimeoutExpired: command exceeded {timeout_seconds} seconds"
        exit_code = 124
    except Exception as exc:
        stderr = f"{type(exc).__name__}: {exc}"
        exit_code = 1

    finished = utc_now_iso()

    block = []
    block.append(f"Finished UTC: `{finished}`  \n")
    block.append(f"Exit code: `{exit_code}`  \n")
    block.append(f"Timed out: `{str(timed_out).lower()}`\n\n")
    block.append("### Command\n\n")
    block.append(fence("text", f"> {redact_api_key(rendered, api_key)}"))
    block.append("\n### stdout\n\n")
    block.append(fence("text", redact_api_key(stdout, api_key)))
    block.append("\n### stderr\n\n")
    block.append(fence("text", redact_api_key(stderr, api_key)))
    append_log(log_path, "".join(block))

    progress("CLI", f"{index:03d} done: exit={exit_code} timeout={timed_out}")

    return {
        "command": list(command),
        "exit_code": exit_code,
        "stdout": stdout,
        "stderr": stderr,
        "timed_out": timed_out,
    }


def run_start_debug_observed(
    index: int,
    log_path: Path,
    command: Sequence[str],
    observe_seconds: int,
    api_key: Optional[str],
    script_dir: Path,
) -> Dict[str, Any]:
    rendered = " ".join(command)
    started = utc_now_iso()

    progress("BROWSER", f"launching debug browser via: {rendered}")
    append_log(
        log_path,
        (
            f"\n## CLI {index:03d}: `{redact_api_key(rendered, api_key)}`\n\n"
            "Note: Start attach-first browser debug through the CLI. This command is observed briefly and will not block the smoke runner indefinitely.\n\n"
            f"Started UTC: `{started}`  \n"
        ),
    )

    stdout_path = script_dir / f".tablinq-step1-start-debug-{local_now_for_filename()}.stdout.log"
    stderr_path = script_dir / f".tablinq-step1-start-debug-{local_now_for_filename()}.stderr.log"

    exit_code: Optional[int] = None
    timed_out = False
    process_pid: Optional[int] = None
    killed_launcher = False
    launch_error = ""

    stdout_handle = None
    stderr_handle = None

    try:
        stdout_handle = stdout_path.open("w", encoding="utf-8", newline="\n")
        stderr_handle = stderr_path.open("w", encoding="utf-8", newline="\n")

        creationflags = 0
        if os.name == "nt":
            creationflags = getattr(subprocess, "CREATE_NEW_PROCESS_GROUP", 0)

        proc = subprocess.Popen(
            list(command),
            stdin=subprocess.DEVNULL,
            stdout=stdout_handle,
            stderr=stderr_handle,
            shell=False,
            creationflags=creationflags,
        )
        process_pid = int(proc.pid)

        try:
            exit_code = proc.wait(timeout=observe_seconds)
            progress("BROWSER", f"start-debug launcher returned with exit={exit_code}")
        except subprocess.TimeoutExpired:
            timed_out = True
            progress("BROWSER", f"start-debug launcher still running after {observe_seconds}s; killing launcher and continuing with endpoint polling")
            try:
                proc.kill()
                killed_launcher = True
                exit_code = 124
            except Exception as kill_exc:
                launch_error = f"Failed to kill start-debug launcher after timeout: {type(kill_exc).__name__}: {kill_exc}"
                exit_code = 124
    except Exception as exc:
        launch_error = f"{type(exc).__name__}: {exc}"
        exit_code = 1
    finally:
        try:
            if stdout_handle:
                stdout_handle.close()
        finally:
            if stderr_handle:
                stderr_handle.close()

    stdout_text = ""
    stderr_text = ""
    if stdout_path.exists():
        stdout_text = stdout_path.read_text(encoding="utf-8", errors="replace")
    if stderr_path.exists():
        stderr_text = stderr_path.read_text(encoding="utf-8", errors="replace")

    finished = utc_now_iso()

    block = []
    block.append(f"Finished/continued UTC: `{finished}`  \n")
    block.append(f"Observed seconds: `{observe_seconds}`  \n")
    block.append(f"Launcher PID: `{process_pid}`  \n")
    block.append(f"Exit code: `{exit_code}`  \n")
    block.append(f"Timed out: `{str(timed_out).lower()}`  \n")
    block.append(f"Killed launcher after timeout: `{str(killed_launcher).lower()}`  \n")
    block.append(f"stdout capture file: `{stdout_path}`  \n")
    block.append(f"stderr capture file: `{stderr_path}`\n\n")
    if launch_error:
        block.append("### Launch error\n\n")
        block.append(fence("text", launch_error))
    block.append("### Command\n\n")
    block.append(fence("text", f"> {redact_api_key(rendered, api_key)}"))
    block.append("\n### stdout captured during observation\n\n")
    block.append(fence("text", redact_api_key(stdout_text, api_key)))
    block.append("\n### stderr captured during observation\n\n")
    block.append(fence("text", redact_api_key(stderr_text, api_key)))
    append_log(log_path, "".join(block))

    return {
        "command": list(command),
        "exit_code": exit_code,
        "timed_out": timed_out,
        "killed_launcher": killed_launcher,
        "stdout": stdout_text,
        "stderr": stderr_text,
        "error": launch_error,
    }


def raw_http_request(
    method: str,
    url: str,
    headers: Optional[Dict[str, str]] = None,
    body_bytes: Optional[bytes] = None,
    timeout_seconds: int = 10,
) -> Dict[str, Any]:
    headers = headers or {}
    started = utc_now_iso()
    status_code = 0
    reason = "TRANSPORT ERROR"
    response_headers = ""
    response_body = ""

    request = urllib.request.Request(url=url, data=body_bytes, headers=headers, method=method.upper())

    try:
        with urllib.request.urlopen(request, timeout=timeout_seconds) as response:
            status_code = int(response.status)
            reason = getattr(response, "reason", "") or ""
            response_headers = response_headers_to_text(status_code, reason, list(response.headers.items()))
            response_body = response.read().decode("utf-8", errors="replace")
    except urllib.error.HTTPError as exc:
        status_code = int(exc.code)
        reason = getattr(exc, "reason", "") or ""
        response_headers = response_headers_to_text(status_code, reason, list(exc.headers.items()) if exc.headers else [])
        try:
            response_body = exc.read().decode("utf-8", errors="replace")
        except Exception as body_exc:
            response_body = f"{type(body_exc).__name__}: failed to read HTTP error body: {body_exc}"
    except Exception as exc:
        response_headers = response_headers_to_text(0, "TRANSPORT ERROR", [])
        response_body = f"{type(exc).__name__}: {exc}"

    return {
        "started": started,
        "finished": utc_now_iso(),
        "status_code": status_code,
        "reason": reason,
        "headers": response_headers,
        "body": response_body,
        "json": try_json(response_body),
    }


def log_debug_probe(
    index: int,
    log_path: Path,
    debug_url: str,
    path: str,
    timeout_seconds: int,
    note: str,
) -> Dict[str, Any]:
    url = debug_url.rstrip("/") + path
    progress("CDP", f"{index:03d} GET {url}")
    result = raw_http_request("GET", url, headers={"Accept": "application/json"}, timeout_seconds=timeout_seconds)

    block = []
    block.append(f"\n## CDP {index:03d}: `GET {path}`\n\n")
    block.append(f"Note: {note}\n\n")
    block.append(f"Started UTC: `{result['started']}`  \n")
    block.append(f"Finished UTC: `{result['finished']}`  \n")
    block.append(f"Status seen by smoke runner: `{result['status_code']}`\n\n")
    block.append("### Raw request\n\n")
    block.append(fence("http", f"GET {url} HTTP/1.1\nAccept: application/json"))
    block.append("\n### Raw response headers\n\n")
    block.append(fence("http", result["headers"]))
    block.append("\n### Raw response body\n\n")
    block.append(fence("text", result["body"]))
    append_log(log_path, "".join(block))

    return result


def poll_debug_endpoint(
    log_path: Path,
    debug_url: str,
    poll_seconds: int,
    interval_seconds: float,
) -> bool:
    progress("CDP", f"polling {debug_url}/json/version for up to {poll_seconds}s")
    append_log(log_path, f"\n## Poll: Chrome debug endpoint\n\nStarted UTC: `{utc_now_iso()}`  \nEndpoint: `{debug_url}/json/version`  \nTimeout seconds: `{poll_seconds}`\n\n")

    deadline = time.monotonic() + poll_seconds
    attempt = 0
    last_status = 0
    last_body = ""

    while time.monotonic() < deadline:
        attempt += 1
        result = raw_http_request("GET", debug_url.rstrip("/") + "/json/version", headers={"Accept": "application/json"}, timeout_seconds=3)
        last_status = int(result["status_code"])
        last_body = str(result["body"])

        if last_status == 200:
            progress("CDP", f"debug endpoint reachable after {attempt} attempt(s)")
            append_log(log_path, f"Debug endpoint reachable after `{attempt}` attempt(s).  \nFinished UTC: `{utc_now_iso()}`\n")
            return True

        progress("CDP", f"waiting for debug endpoint; attempt={attempt} status={last_status}")
        time.sleep(interval_seconds)

    append_log(log_path, f"Debug endpoint was not reachable. Last status: `{last_status}`  \nLast body/error: `{last_body[:1000]}`  \nFinished UTC: `{utc_now_iso()}`\n")
    progress("CDP", "debug endpoint was not reachable before timeout")
    return False


def send_api_request(
    index: int,
    log_path: Path,
    method: str,
    root_base_url: str,
    path: str,
    api_key: str,
    timeout_seconds: int,
    note: str,
    body_text: Optional[str] = None,
) -> Dict[str, Any]:
    method = method.upper()
    if not path.startswith("/"):
        path = "/" + path

    url = root_base_url + path
    body_bytes = body_text.encode("utf-8") if body_text is not None else None

    headers: Dict[str, str] = {
        "Accept": "application/json",
        "Authorization": f"Bearer {api_key}",
    }
    if body_bytes is not None:
        headers["Content-Type"] = "application/json"

    curl_headers = dict(headers)
    curl_headers["Authorization"] = "Bearer <REDACTED_API_KEY>"
    curl_text = build_curl(method, url, curl_headers, body_bytes)

    progress("API", f"{index:03d} {method} {path}")
    result = raw_http_request(method, url, headers=headers, body_bytes=body_bytes, timeout_seconds=timeout_seconds)

    block = []
    block.append(f"\n## API {index:03d}: `{method} {path}`\n\n")
    block.append(f"Note: {note}\n\n")
    block.append(f"Started UTC: `{result['started']}`  \n")
    block.append(f"Finished UTC: `{result['finished']}`  \n")
    block.append(f"Status seen by smoke runner: `{result['status_code']}`\n\n")
    block.append("### cURL\n\n")
    block.append(fence("bash", curl_text))
    block.append("\n### Raw response headers\n\n")
    block.append(fence("http", redact_api_key(result["headers"], api_key)))
    block.append("\n### Raw response body\n\n")
    block.append(fence("text", redact_api_key(result["body"], api_key)))
    append_log(log_path, "".join(block))

    progress("API", f"{index:03d} done: status={result['status_code']}")
    return result


def boolish(value: Any) -> Optional[bool]:
    if isinstance(value, bool):
        return value
    return None


def summarize_status(api_results: Dict[str, Dict[str, Any]], cdp_reachable: bool) -> Tuple[str, List[str], Dict[str, Any]]:
    admin_json = None
    for key in ("/admin/status#after", "/admin/status#before", "/v1/status#after", "/v1/status#before"):
        candidate = api_results.get(key, {}).get("json")
        if isinstance(candidate, dict):
            admin_json = candidate
            break

    ready_json = api_results.get("/ready#after", {}).get("json") or api_results.get("/ready#before", {}).get("json")

    facts: Dict[str, Any] = {
        "cdp_reachable": cdp_reachable,
        "gateway_ready": None,
        "gateway_running": None,
        "browser_connected": None,
        "extension_connected": None,
        "authenticated": None,
        "target_tab_found": None,
        "page_ready": None,
        "health_status": None,
    }

    if isinstance(ready_json, dict):
        facts["gateway_ready"] = boolish(ready_json.get("ready"))

    if isinstance(admin_json, dict):
        facts["gateway_running"] = boolish(find_first_key(admin_json, "running"))
        facts["browser_connected"] = boolish(find_first_key(admin_json, "browser_connected"))
        facts["extension_connected"] = boolish(find_first_key(admin_json, "extension_connected"))
        facts["authenticated"] = boolish(find_first_key(admin_json, "authenticated"))
        facts["target_tab_found"] = boolish(find_first_key(admin_json, "target_tab_found"))
        facts["page_ready"] = boolish(find_first_key(admin_json, "page_ready"))
        status = find_first_key(admin_json, "status")
        if isinstance(status, str):
            facts["health_status"] = status

        if facts["gateway_ready"] is None:
            facts["gateway_ready"] = boolish(find_first_key(admin_json, "ready"))

    problems: List[str] = []

    if facts["gateway_ready"] is not True:
        problems.append("gateway readiness is not confirmed true")
    if facts["gateway_running"] is not True:
        problems.append("gateway running state is not confirmed true")
    if facts["cdp_reachable"] is not True:
        problems.append("Chrome debug endpoint is not reachable")
    if facts["browser_connected"] is not True:
        problems.append("browser connection is not confirmed true")
    if facts["authenticated"] is not True:
        problems.append("ChatGPT authentication is not confirmed true")
    if facts["extension_connected"] is not True:
        problems.append("Tablinq extension connection is not confirmed true")
    if facts["target_tab_found"] is not True:
        problems.append("target ChatGPT tab is not confirmed found")
    if facts["page_ready"] is not True:
        problems.append("page readiness is not confirmed true")

    if problems:
        return "degraded", problems, facts
    return "healthy", [], facts


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Tablinq gateway step 1 readiness smoke logger, v2.")
    parser.add_argument("--env-file", default=None, help="Path to .env.local. Defaults to .env.local beside this script.")
    parser.add_argument("--base-url", default=None, help="Gateway root base URL, e.g. http://127.0.0.1:3847. /v1 suffix is accepted and stripped.")
    parser.add_argument("--debug-url", default=None, help="Chrome DevTools debug endpoint. Defaults to http://127.0.0.1:9222.")
    parser.add_argument("--log-file", default=None, help="Markdown log file path. Defaults to a timestamped file beside this script.")
    parser.add_argument("--gpt-bin", default=os.environ.get("TABLINQ_GPT_BIN", "gpt"), help="gpt CLI executable name or path.")
    parser.add_argument("--timeout", type=int, default=30, help="Normal per-command and per-request timeout in seconds.")
    parser.add_argument("--start-debug-observe-seconds", type=int, default=15, help="Seconds to observe gpt browser start-debug before killing only the launcher and continuing.")
    parser.add_argument("--debug-poll-seconds", type=int, default=90, help="Seconds to wait for the Chrome debug endpoint after launch.")
    parser.add_argument("--attach-timeout", type=int, default=120, help="Timeout for gpt browser attach.")
    parser.add_argument("--skip-cli", action="store_true", help="Skip CLI commands and only run gateway HTTP checks.")
    parser.add_argument("--skip-start-debug", action="store_true", help="Skip gpt browser start-debug.")
    parser.add_argument("--force-start-debug", action="store_true", help="Run gpt browser start-debug even if the debug endpoint is already reachable.")
    parser.add_argument("--no-control-plane-start", action="store_true", help="Do not run gpt daemon start and gpt gateway start.")
    parser.add_argument("--exit-zero-on-degraded", action="store_true", help="Return exit code 0 even when final verdict is degraded.")
    return parser


def main(argv: Optional[Sequence[str]] = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)

    script_dir = Path(__file__).resolve().parent
    env_file = Path(args.env_file).expanduser().resolve() if args.env_file else script_dir / ".env.local"
    env_values = parse_env_file(env_file)
    for key, value in env_values.items():
        os.environ.setdefault(key, value)

    api_key_name, api_key = first_present_env(API_KEY_ENV_NAMES, env_values)
    _base_name, base_url_from_env = first_present_env(BASE_URL_ENV_NAMES, env_values)
    _debug_name, debug_url_from_env = first_present_env(DEBUG_URL_ENV_NAMES, env_values)

    root_base_url = normalize_root_base_url(args.base_url or base_url_from_env or DEFAULT_ROOT_BASE_URL)
    debug_url = normalize_debug_url(args.debug_url or debug_url_from_env or DEFAULT_DEBUG_URL)

    if args.log_file:
        log_path = Path(args.log_file).expanduser().resolve()
    else:
        log_path = script_dir / f"tablinq-step1-readiness-v2-{local_now_for_filename()}.md"

    log_path.parent.mkdir(parents=True, exist_ok=True)
    if log_path.exists():
        log_path.unlink()

    progress("SETUP", f"log file: {log_path}")
    progress("SETUP", f"env file: {env_file}")
    progress("SETUP", f"gateway URL: {root_base_url}")
    progress("SETUP", f"debug URL: {debug_url}")

    header = []
    header.append("# Tablinq Gateway Step 1 Readiness Smoke Log v2\n\n")
    header.append(f"Generated UTC: `{utc_now_iso()}`  \n")
    header.append(f"Runner: `{Path(__file__).name}`  \n")
    header.append(f"Python: `{sys.version.split()[0]}`  \n")
    header.append(f"Working directory: `{Path.cwd()}`  \n")
    header.append(f".env.local path: `{env_file}`  \n")
    header.append(f"Gateway root base URL: `{root_base_url}`  \n")
    header.append(f"Chrome debug URL: `{debug_url}`  \n")
    header.append(f"API key source: `{api_key_name or 'NOT FOUND'}`  \n")
    header.append("API key value: `<REDACTED_API_KEY>`  \n\n")
    header.append("Scope: step 1 readiness only. This run does not send chat/completions prompts, does not call research, and does not process a prior markdown log.\n")
    header.append("\nImportant behavior: `gpt browser start-debug` is observed briefly, then the smoke runner polls the debug endpoint and continues. The runner does not wait indefinitely for the launcher command.\n")
    append_log(log_path, "".join(header))

    final_verdict = "runner_failed"
    final_problems: List[str] = []
    final_facts: Dict[str, Any] = {}
    api_results: Dict[str, Dict[str, Any]] = {}
    cdp_reachable = False
    exit_code = 1

    try:
        if not api_key:
            raise RuntimeError("No gateway API key found. Expected one of: " + ", ".join(API_KEY_ENV_NAMES))

        cli_index = 1

        if not args.skip_cli:
            pre_commands: List[Tuple[List[str], int, str]] = []
            if os.name == "nt":
                pre_commands.append((["where", args.gpt_bin], args.timeout, "Resolve the gpt CLI on PATH before using it."))
            else:
                pre_commands.append((["which", args.gpt_bin], args.timeout, "Resolve the gpt CLI on PATH before using it."))

            pre_commands.append(([args.gpt_bin, "version"], args.timeout, "Capture installed CLI version/build."))
            if not args.no_control_plane_start:
                pre_commands.append(([args.gpt_bin, "daemon", "start"], args.timeout, "Ensure daemon is running after install."))
                pre_commands.append(([args.gpt_bin, "gateway", "start"], args.timeout, "Ensure gateway is running after install."))
            pre_commands.append(([args.gpt_bin, "daemon", "status"], args.timeout, "Capture daemon status."))
            pre_commands.append(([args.gpt_bin, "gateway", "status"], args.timeout, "Capture gateway status before browser launch/attach."))
            pre_commands.append(([args.gpt_bin, "status"], args.timeout, "Capture overall Tablinq status before browser launch/attach."))
            pre_commands.append(([args.gpt_bin, "doctor"], args.timeout, "Capture doctor output before browser launch/attach."))
            pre_commands.append(([args.gpt_bin, "config", "show"], args.timeout, "Capture effective CLI-visible config."))
            pre_commands.append(([args.gpt_bin, "browser", "connection-info"], args.timeout, "Capture browser debug endpoint and attach profile information before start-debug."))
            pre_commands.append(([args.gpt_bin, "browser", "profile-info"], args.timeout, "Capture browser profile state before start-debug."))

            for command, timeout_seconds, note in pre_commands:
                run_command_capture(cli_index, log_path, command, timeout_seconds, api_key, note)
                cli_index += 1

        first_probe = log_debug_probe(1, log_path, debug_url, "/json/version", 5, "Probe Chrome DevTools endpoint before deciding whether to launch start-debug.")
        endpoint_already_reachable = int(first_probe["status_code"]) == 200

        if not args.skip_cli and not args.skip_start_debug:
            if endpoint_already_reachable and not args.force_start_debug:
                progress("BROWSER", "debug endpoint already reachable; not launching another start-debug process")
                append_log(log_path, "\n## Browser start-debug decision\n\nDebug endpoint was already reachable, so `gpt browser start-debug` was not launched again. Use `-ForceStartDebug` in the PS1 launcher to override this.\n")
            else:
                run_start_debug_observed(
                    cli_index,
                    log_path,
                    [args.gpt_bin, "browser", "start-debug"],
                    args.start_debug_observe_seconds,
                    api_key,
                    script_dir,
                )
                cli_index += 1

        cdp_reachable = poll_debug_endpoint(log_path, debug_url, args.debug_poll_seconds, 3.0)
        log_debug_probe(2, log_path, debug_url, "/json/version", 5, "Capture final Chrome DevTools version response after start-debug/polling.")
        log_debug_probe(3, log_path, debug_url, "/json/list", 5, "Capture visible debug targets after start-debug/polling.")

        if not args.skip_cli:
            run_command_capture(cli_index, log_path, [args.gpt_bin, "browser", "connection-info"], args.timeout, api_key, "Capture browser connection-info after debug endpoint polling.")
            cli_index += 1
            run_command_capture(cli_index, log_path, [args.gpt_bin, "browser", "attach"], args.attach_timeout, api_key, "Attach Tablinq to the debug browser session through the CLI.")
            cli_index += 1
            run_command_capture(cli_index, log_path, [args.gpt_bin, "browser", "profile-info"], args.timeout, api_key, "Capture browser profile-info after CLI attach.")
            cli_index += 1
            run_command_capture(cli_index, log_path, [args.gpt_bin, "status"], args.timeout, api_key, "Capture overall status after CLI attach.")
            cli_index += 1
            run_command_capture(cli_index, log_path, [args.gpt_bin, "doctor"], args.timeout, api_key, "Capture doctor output after CLI attach.")
            cli_index += 1

        before_api_requests: List[Tuple[str, str, str, Optional[str], str]] = [
            ("GET", "/health", "Gateway process health check before API attach.", None, "/health#before"),
            ("GET", "/ready", "Gateway readiness check before API attach.", None, "/ready#before"),
            ("GET", "/admin/status", "Gateway admin status snapshot before API attach.", None, "/admin/status#before"),
            ("GET", "/v1/status", "OpenAI-compatible gateway status before API attach.", None, "/v1/status#before"),
            ("GET", "/v1/doctor", "Gateway doctor report before API attach.", None, "/v1/doctor#before"),
            ("GET", "/v1/config", "Gateway config surface before API attach.", None, "/v1/config#before"),
            ("GET", "/v1/browser/connection-info", "Browser connection information before API attach.", None, "/v1/browser/connection-info#before"),
            ("GET", "/v1/browser/profile-info", "Browser profile information before API attach.", None, "/v1/browser/profile-info#before"),
        ]

        api_index = 1
        for method, path, note, body, key in before_api_requests:
            api_results[key] = send_api_request(api_index, log_path, method, root_base_url, path, api_key, args.timeout, note, body)
            api_index += 1

        api_results["/v1/browser/attach#call"] = send_api_request(api_index, log_path, "POST", root_base_url, "/v1/browser/attach", api_key, args.attach_timeout, "Attach gateway/core to the debug browser session through the HTTP API.", "{}")
        api_index += 1

        after_api_requests: List[Tuple[str, str, str, Optional[str], str]] = [
            ("GET", "/v1/browser/connection-info", "Browser connection information after API attach.", None, "/v1/browser/connection-info#after"),
            ("GET", "/v1/browser/profile-info", "Browser profile information after API attach.", None, "/v1/browser/profile-info#after"),
            ("GET", "/ready", "Gateway readiness check after API attach.", None, "/ready#after"),
            ("GET", "/admin/status", "Gateway admin status snapshot after API attach.", None, "/admin/status#after"),
            ("GET", "/v1/status", "OpenAI-compatible gateway status after API attach.", None, "/v1/status#after"),
            ("GET", "/v1/doctor", "Gateway doctor report after API attach.", None, "/v1/doctor#after"),
            ("GET", "/v1/config", "Gateway config surface after API attach.", None, "/v1/config#after"),
        ]

        for method, path, note, body, key in after_api_requests:
            api_results[key] = send_api_request(api_index, log_path, method, root_base_url, path, api_key, args.timeout, note, body)
            api_index += 1

        final_verdict, final_problems, final_facts = summarize_status(api_results, cdp_reachable)
        exit_code = 0 if final_verdict == "healthy" or args.exit_zero_on_degraded else 3

    except KeyboardInterrupt:
        final_verdict = "interrupted"
        final_problems = ["runner was interrupted"]
        exit_code = 130
        append_log(log_path, "\n## Runner interruption\n\nThe runner received KeyboardInterrupt.\n")
        progress("ERROR", "runner interrupted")
    except Exception as exc:
        final_verdict = "runner_failed"
        final_problems = [f"{type(exc).__name__}: {exc}"]
        exit_code = 1
        append_log(log_path, "\n## Runner exception\n\n" + fence("text", traceback.format_exc()))
        progress("ERROR", f"{type(exc).__name__}: {exc}")
    finally:
        progress("SUMMARY", f"verdict={final_verdict}")
        if final_problems:
            for problem in final_problems:
                progress("SUMMARY", f"problem: {problem}")

        summary = []
        summary.append("\n## Final step 1 verdict\n\n")
        summary.append(f"Finished UTC: `{utc_now_iso()}`  \n")
        summary.append(f"Verdict: `{final_verdict}`  \n")
        summary.append(f"Runner exit code: `{exit_code}`\n\n")
        summary.append("### Extracted status fields\n\n")
        summary.append(fence("json", json.dumps(final_facts, indent=2, sort_keys=True)))
        summary.append("\n### Problems\n\n")
        if final_problems:
            for problem in final_problems:
                summary.append(f"- {problem}\n")
        else:
            summary.append("- none\n")
        append_log(log_path, "".join(summary))
        progress("SUMMARY", f"log file: {log_path}")

    return exit_code


if __name__ == "__main__":
    raise SystemExit(main())
