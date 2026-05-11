#!/usr/bin/env python3
"""
Tablinq gateway step 2 control-plane smoke v2.

Required behavior:
- No /v1/chat/completions calls.
- No prompt submission.
- No deep research.
- No valid destructive admin calls.
- Logs raw redacted cURL, raw response headers, raw response bodies, and short notes to one markdown file.
- Retries transport errors only, preserving all attempts.
- Requires browser attach-state convergence after every successful API attach.
"""

from __future__ import annotations

import argparse
import datetime as _dt
import json
import os
import sys
import time
import traceback
import urllib.error
import urllib.parse
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

DEFAULT_ROOT_BASE_URL = "http://127.0.0.1:3847"


class SmokeStats:
    def __init__(self) -> None:
        self.total_requests = 0
        self.http_status_counts: Dict[str, int] = {}
        self.transport_first_attempt_failures = 0
        self.transport_final_failures = 0
        self.retry_recovered = 0
        self.routes_with_transport_warnings: List[str] = []
        self.semantic_failures: List[str] = []
        self.semantic_warnings: List[str] = []

    def record(self, route_label: str, result: Dict[str, Any]) -> None:
        self.total_requests += 1
        status = str(result.get("status_code", 0))
        self.http_status_counts[status] = self.http_status_counts.get(status, 0) + 1

        attempts = result.get("attempts") or [result]
        first = attempts[0]
        first_status = int(first.get("status_code", 0) or 0)
        final_status = int(result.get("status_code", 0) or 0)

        if first_status == 0:
            self.transport_first_attempt_failures += 1
            self.routes_with_transport_warnings.append(route_label)

        if final_status == 0:
            self.transport_final_failures += 1
        elif first_status == 0:
            self.retry_recovered += 1

    def as_dict(self) -> Dict[str, Any]:
        return {
            "total_requests": self.total_requests,
            "http_status_counts": self.http_status_counts,
            "transport_first_attempt_failures": self.transport_first_attempt_failures,
            "transport_final_failures": self.transport_final_failures,
            "retry_recovered": self.retry_recovered,
            "routes_with_transport_warnings": self.routes_with_transport_warnings,
            "semantic_failures": self.semantic_failures,
            "semantic_warnings": self.semantic_warnings,
        }


def now_local() -> str:
    return _dt.datetime.now().strftime("%H:%M:%S")


def utc_now_iso() -> str:
    return _dt.datetime.now(_dt.timezone.utc).isoformat(timespec="seconds")


def filename_stamp() -> str:
    return _dt.datetime.now().strftime("%Y%m%d-%H%M%S")


def progress(phase: str, message: str) -> None:
    print(f"[{now_local()}] {phase}: {message}", flush=True)


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


def first_present(names: Iterable[str], env_values: Dict[str, str]) -> Tuple[Optional[str], Optional[str]]:
    for name in names:
        value = os.environ.get(name)
        if value and value.strip():
            return name, value.strip()

    for name in names:
        value = env_values.get(name)
        if value and value.strip():
            return name, value.strip()

    return None, None


def normalize_root_base_url(base_url: str) -> str:
    base_url = base_url.strip().rstrip("/")
    if base_url.endswith("/v1"):
        base_url = base_url[:-3].rstrip("/")
    return base_url or DEFAULT_ROOT_BASE_URL


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


def redact(text: str, api_key: Optional[str]) -> str:
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


def parse_json(body: str) -> Optional[Any]:
    try:
        return json.loads(body)
    except Exception:
        return None


def raw_http(
    method: str,
    url: str,
    headers: Dict[str, str],
    body_bytes: Optional[bytes],
    timeout_seconds: int,
) -> Dict[str, Any]:
    started = utc_now_iso()
    status_code = 0
    reason = "TRANSPORT ERROR"
    response_headers = ""
    response_body = ""
    error_type = ""

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
        error_type = type(exc).__name__
        response_headers = response_headers_to_text(0, "TRANSPORT ERROR", [])
        response_body = f"{type(exc).__name__}: {exc}"

    return {
        "started": started,
        "finished": utc_now_iso(),
        "status_code": status_code,
        "reason": reason,
        "headers": response_headers,
        "body": response_body,
        "json": parse_json(response_body),
        "transport_error_type": error_type,
    }


def absolute_url(root_base_url: str, path: str) -> str:
    if path.startswith("http://") or path.startswith("https://"):
        return path
    if not path.startswith("/"):
        path = "/" + path
    return root_base_url + path


def request_with_logging(
    log_path: Path,
    stats: SmokeStats,
    index: int,
    section: str,
    method: str,
    root_base_url: str,
    path: str,
    api_key: str,
    timeout_seconds: int,
    retries: int,
    retry_delay_seconds: float,
    note: str,
    body_text: Optional[str] = None,
    auth_mode: str = "valid",
    extra_headers: Optional[Dict[str, str]] = None,
    content_type: Optional[str] = None,
) -> Dict[str, Any]:
    method = method.upper()
    url = absolute_url(root_base_url, path)
    body_bytes = body_text.encode("utf-8") if body_text is not None else None

    headers: Dict[str, str] = {"Accept": "application/json"}

    if auth_mode == "valid":
        headers["Authorization"] = f"Bearer {api_key}"
    elif auth_mode == "missing":
        pass
    elif auth_mode == "wrong":
        headers["Authorization"] = "Bearer definitely-wrong-step2-token"
    elif auth_mode == "malformed":
        headers["Authorization"] = "Bearer"
    elif auth_mode.startswith("literal:"):
        headers["Authorization"] = auth_mode[len("literal:") :]
    else:
        raise ValueError(f"unknown auth_mode: {auth_mode}")

    if body_bytes is not None:
        headers["Content-Type"] = content_type or "application/json"

    if extra_headers:
        headers.update(extra_headers)

    curl_headers = dict(headers)
    if curl_headers.get("Authorization") == f"Bearer {api_key}":
        curl_headers["Authorization"] = "Bearer <REDACTED_API_KEY>"

    curl_text = build_curl(method, url, curl_headers, body_bytes)

    append_log(log_path, f"\n## {section} {index:03d}: `{method} {path}`\n\n")
    append_log(log_path, f"Note: {note}\n\n")
    append_log(log_path, f"Auth mode: `{auth_mode}`  \n")
    append_log(log_path, f"Retries for transport errors: `{retries}`\n\n")
    append_log(log_path, "### cURL\n\n")
    append_log(log_path, fence("bash", redact(curl_text, api_key)))

    attempts: List[Dict[str, Any]] = []

    for attempt in range(1, retries + 2):
        progress(section, f"{index:03d} attempt {attempt}/{retries + 1}: {method} {path}")
        result = raw_http(method, url, headers, body_bytes, timeout_seconds)
        attempts.append(result)

        append_log(log_path, f"\n### Attempt {attempt}\n\n")
        append_log(log_path, f"Started UTC: `{result['started']}`  \n")
        append_log(log_path, f"Finished UTC: `{result['finished']}`  \n")
        append_log(log_path, f"Status seen by smoke runner: `{result['status_code']}`  \n")
        append_log(log_path, f"Transport error type: `{result['transport_error_type']}`\n\n")
        append_log(log_path, "#### Raw response headers\n\n")
        append_log(log_path, fence("http", redact(result["headers"], api_key)))
        append_log(log_path, "\n#### Raw response body\n\n")
        append_log(log_path, fence("text", redact(result["body"], api_key)))

        if int(result["status_code"]) != 0:
            break

        if attempt <= retries:
            append_log(log_path, f"\nTransport error retry delay seconds: `{retry_delay_seconds}`\n")
            progress(section, f"{index:03d} transport error; retrying after {retry_delay_seconds}s")
            time.sleep(retry_delay_seconds)

    final = attempts[-1]
    final["attempts"] = attempts
    final["attempt_count"] = len(attempts)
    final["retried"] = len(attempts) > 1

    stats.record(f"{section} {index:03d} {method} {path}", final)
    return final


def get_path(value: Any, path: str) -> Any:
    node = value
    for part in path.split("."):
        if isinstance(node, dict) and part in node:
            node = node[part]
        else:
            return None
    return node


def endpoint_attached_state(path: str, body_json: Any) -> Tuple[bool, Dict[str, Any], List[str]]:
    fields: Dict[str, Any] = {}
    failures: List[str] = []

    if not isinstance(body_json, dict):
        return False, {"json_object": False}, ["response body is not a JSON object"]

    if path == "/health":
        expected = {
            "status": "ok",
            "health.status": "ok",
            "health.authenticated": True,
            "health.browser_connected": True,
            "health.extension_connected": True,
        }
    elif path == "/admin/status":
        expected = {
            "health.status": "ok",
            "health.authenticated": True,
            "health.browser_connected": True,
            "health.extension_connected": True,
            "browser.authenticated": True,
            "browser.cdp_attached": True,
            "browser.extension_connected": True,
            "browser.page_ready": True,
            "browser.running": True,
            "browser.target_tab_found": True,
        }
    elif path == "/v1/browser/profile-info":
        expected = {
            "data.auth_source": "page",
            "data.authenticated": True,
            "data.cdp_attached": True,
            "data.extension_connected": True,
            "data.page_ready": True,
            "data.running": True,
            "data.target_tab_found": True,
        }
    elif path == "/v1/browser/connection-info":
        expected = {
            "browser.auth_source": "page",
            "browser.authenticated": True,
            "browser.cdp_attached": True,
            "browser.extension_connected": True,
            "browser.page_ready": True,
            "browser.running": True,
            "browser.target_tab_found": True,
        }
    else:
        return False, {"unknown_endpoint": path}, [f"unknown convergence endpoint: {path}"]

    for key_path, expected_value in expected.items():
        actual_value = get_path(body_json, key_path)
        fields[key_path] = actual_value
        if actual_value != expected_value:
            failures.append(f"{path}: expected {key_path}={expected_value!r}, got {actual_value!r}")

    return len(failures) == 0, fields, failures


def wait_for_attach_convergence(
    log_path: Path,
    stats: SmokeStats,
    next_index: int,
    label: str,
    root_base_url: str,
    api_key: str,
    timeout_seconds: int,
    retries: int,
    retry_delay_seconds: float,
    convergence_timeout_seconds: int,
    convergence_interval_seconds: float,
) -> Tuple[int, bool, List[str], Dict[str, Any]]:
    endpoints = [
        "/health",
        "/admin/status",
        "/v1/browser/profile-info",
        "/v1/browser/connection-info",
    ]

    append_log(log_path, f"\n# Attach convergence check: {label}\n\n")
    append_log(log_path, "Required semantic state: all convergence endpoints must agree that the browser is attached, authenticated, CDP-attached where applicable, extension-connected, page-ready, running, and target tab found.\n\n")
    append_log(log_path, f"Convergence timeout seconds: `{convergence_timeout_seconds}`  \n")
    append_log(log_path, f"Convergence interval seconds: `{convergence_interval_seconds}`\n\n")

    deadline = time.monotonic() + convergence_timeout_seconds
    round_no = 0
    last_failures: List[str] = []
    last_observed: Dict[str, Any] = {}

    while time.monotonic() < deadline:
        round_no += 1
        progress("CONVERGE", f"{label}: round {round_no}")
        append_log(log_path, f"\n## Convergence round {round_no}: {label}\n\n")

        all_ok = True
        round_failures: List[str] = []
        round_observed: Dict[str, Any] = {}

        for endpoint in endpoints:
            result = request_with_logging(
                log_path=log_path,
                stats=stats,
                index=next_index,
                section="CONVERGE",
                method="GET",
                root_base_url=root_base_url,
                path=endpoint,
                api_key=api_key,
                timeout_seconds=timeout_seconds,
                retries=retries,
                retry_delay_seconds=retry_delay_seconds,
                note=f"Attach convergence check for {endpoint} during {label}.",
            )
            next_index += 1

            if int(result.get("status_code", 0) or 0) != 200:
                all_ok = False
                failure = f"{endpoint}: expected HTTP 200 during convergence, got {result.get('status_code')}"
                round_failures.append(failure)
                round_observed[endpoint] = {"status_code": result.get("status_code"), "fields": None}
                continue

            ok, fields, failures = endpoint_attached_state(endpoint, result.get("json"))
            round_observed[endpoint] = {"status_code": result.get("status_code"), "fields": fields, "ok": ok}
            if not ok:
                all_ok = False
                round_failures.extend(failures)

        append_log(log_path, "\n### Extracted convergence fields\n\n")
        append_log(log_path, fence("json", json.dumps(round_observed, indent=2, sort_keys=True)))
        if round_failures:
            append_log(log_path, "\n### Convergence failures this round\n\n")
            for failure in round_failures:
                append_log(log_path, f"- {failure}\n")
        else:
            append_log(log_path, "\n### Convergence failures this round\n\n- none\n")

        last_failures = round_failures
        last_observed = round_observed

        if all_ok:
            progress("CONVERGE", f"{label}: converged after {round_no} round(s)")
            append_log(log_path, f"\nConvergence result for `{label}`: `ok` after `{round_no}` round(s).\n")
            return next_index, True, [], last_observed

        time.sleep(convergence_interval_seconds)

    progress("CONVERGE", f"{label}: failed to converge before timeout")
    append_log(log_path, f"\nConvergence result for `{label}`: `failed` after `{round_no}` round(s).\n")
    return next_index, False, last_failures, last_observed


def collect_string_ids(value: Any, key_names: Iterable[str]) -> List[str]:
    keys = set(key_names)
    found: List[str] = []

    def walk(node: Any) -> None:
        if isinstance(node, dict):
            for key, child in node.items():
                if key in keys and isinstance(child, str) and child.strip():
                    found.append(child.strip())
                walk(child)
        elif isinstance(node, list):
            for child in node:
                walk(child)

    walk(value)

    deduped: List[str] = []
    seen = set()
    for item in found:
        if item not in seen:
            seen.add(item)
            deduped.append(item)
    return deduped


def id_segment(value: str) -> str:
    return urllib.parse.quote(value, safe="")


def add_header(log_path: Path, env_file: Path, root_base_url: str, api_key_name: str, args: argparse.Namespace) -> None:
    append_log(log_path, "# Tablinq Gateway Step 2 Control-Plane Smoke Log v2\n\n")
    append_log(log_path, f"Generated UTC: `{utc_now_iso()}`  \n")
    append_log(log_path, f"Runner: `tablinq_step2_control_plane.py`  \n")
    append_log(log_path, f"Python: `{sys.version.split()[0]}`  \n")
    append_log(log_path, f"Working directory: `{Path.cwd()}`  \n")
    append_log(log_path, f".env.local path: `{env_file}`  \n")
    append_log(log_path, f"Gateway root base URL: `{root_base_url}`  \n")
    append_log(log_path, f"API key source: `{api_key_name}`  \n")
    append_log(log_path, "API key value: `<REDACTED_API_KEY>`  \n\n")
    append_log(log_path, "Scope: step 2 control-plane, metadata, discovery, dynamic read, auth, and route-contract smoke only. No chat/completions calls, no prompt submission, no deep research, no valid destructive admin calls.\n\n")
    append_log(log_path, "Required starting state: install completed and Chrome debug browser is already running. This script performs API attach and requires convergence. It does not run `gpt browser start-debug`.\n\n")
    append_log(log_path, f"Attach convergence timeout seconds: `{args.attach_convergence_timeout}`  \n")
    append_log(log_path, f"Attach convergence interval seconds: `{args.attach_convergence_interval}`\n")


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Tablinq step 2 control-plane smoke with required attach convergence.")
    parser.add_argument("--env-file", default=None, help="Path to .env.local. Defaults to .env.local beside this script.")
    parser.add_argument("--base-url", default=None, help="Gateway root base URL. Defaults to env or http://127.0.0.1:3847.")
    parser.add_argument("--log-file", default=None, help="Markdown log path. Defaults to timestamped file beside this script.")
    parser.add_argument("--timeout", type=int, default=25, help="Per-request timeout seconds.")
    parser.add_argument("--retries", type=int, default=2, help="Transport retries per request. HTTP responses are not retried.")
    parser.add_argument("--retry-delay", type=float, default=0.75, help="Delay between transport retries.")
    parser.add_argument("--stability-loops", type=int, default=3, help="Repeated stability loops for noisy routes after convergence.")
    parser.add_argument("--skip-detach-cycle", action="store_true", help="Skip POST /v1/browser/detach and restore attach.")
    parser.add_argument("--attach-convergence-timeout", type=int, default=35, help="Seconds to wait for endpoint convergence after each API attach.")
    parser.add_argument("--attach-convergence-interval", type=float, default=2.0, help="Seconds between convergence rounds.")
    parser.add_argument("--exit-zero-on-warnings", action="store_true", help="Return 0 even if warnings occurred.")
    return parser


def main(argv: Optional[Sequence[str]] = None) -> int:
    args = build_parser().parse_args(argv)

    script_dir = Path(__file__).resolve().parent
    env_file = Path(args.env_file).expanduser().resolve() if args.env_file else script_dir / ".env.local"
    env_values = parse_env_file(env_file)

    for key, value in env_values.items():
        os.environ.setdefault(key, value)

    api_key_name, api_key = first_present(API_KEY_ENV_NAMES, env_values)
    _base_name, base_url_from_env = first_present(BASE_URL_ENV_NAMES, env_values)

    root_base_url = normalize_root_base_url(args.base_url or base_url_from_env or DEFAULT_ROOT_BASE_URL)

    if args.log_file:
        log_path = Path(args.log_file).expanduser().resolve()
    else:
        log_path = script_dir / f"tablinq-step2-control-plane-v2-{filename_stamp()}.md"

    if log_path.exists():
        log_path.unlink()

    stats = SmokeStats()
    verdict = "runner_failed"
    problems: List[str] = []
    warnings: List[str] = []

    try:
        if not api_key:
            raise RuntimeError("No API key found. Expected one of: " + ", ".join(API_KEY_ENV_NAMES))

        progress("SETUP", f"log file: {log_path}")
        progress("SETUP", f"env file: {env_file}")
        progress("SETUP", f"gateway URL: {root_base_url}")
        add_header(log_path, env_file, root_base_url, api_key_name or "unknown", args)

        idx = 1

        append_log(log_path, "\n# A. Preflight status before required API attach\n\n")
        append_log(log_path, "Preflight may be degraded if the browser is not currently attached. Step 2 v2 does not decide readiness here; it requires convergence after API attach.\n")
        preflight = [
            ("GET", "/health", "Pre-attach gateway health snapshot.", None),
            ("GET", "/ready", "Gateway readiness.", None),
            ("GET", "/admin/status", "Pre-attach admin status snapshot.", None),
            ("GET", "/v1/status", "OpenAI-compatible gateway status.", None),
            ("GET", "/v1/doctor", "Gateway doctor report.", None),
            ("GET", "/v1/config", "Gateway config. Included because prior runs saw transport noise here.", None),
        ]
        for method, path, note, body in preflight:
            request_with_logging(log_path, stats, idx, "PREFLIGHT", method, root_base_url, path, api_key, args.timeout, args.retries, args.retry_delay, note, body)
            idx += 1

        append_log(log_path, "\n# B. Required initial API attach and convergence\n\n")
        request_with_logging(log_path, stats, idx, "ATTACH", "POST", root_base_url, "/v1/browser/attach", api_key, args.timeout, args.retries, args.retry_delay, "Required initial API attach. The following convergence check must pass.", "{}")
        idx += 1

        idx, converged, failures, _observed = wait_for_attach_convergence(
            log_path=log_path,
            stats=stats,
            next_index=idx,
            label="initial API attach",
            root_base_url=root_base_url,
            api_key=api_key,
            timeout_seconds=args.timeout,
            retries=args.retries,
            retry_delay_seconds=args.retry_delay,
            convergence_timeout_seconds=args.attach_convergence_timeout,
            convergence_interval_seconds=args.attach_convergence_interval,
        )
        if not converged:
            stats.semantic_failures.extend([f"initial API attach convergence failed: {failure}" for failure in failures])

        append_log(log_path, "\n# C. Browser route sweep after required convergence attempt\n\n")
        browser_routes = [
            ("GET", "/v1/browser/connection-info", "Browser connection info after initial attach/convergence.", None),
            ("GET", "/v1/browser/profile-info", "Browser profile info after initial attach/convergence.", None),
            ("GET", "/admin/status", "Admin status after initial attach/convergence.", None),
            ("GET", "/health", "Health after initial attach/convergence.", None),
        ]
        for method, path, note, body in browser_routes:
            request_with_logging(log_path, stats, idx, "BROWSER", method, root_base_url, path, api_key, args.timeout, args.retries, args.retry_delay, note, body)
            idx += 1

        if not args.skip_detach_cycle:
            append_log(log_path, "\n# D. Detach/reattach cycle with required convergence\n\n")
            request_with_logging(log_path, stats, idx, "DETACH", "POST", root_base_url, "/v1/browser/detach", api_key, args.timeout, args.retries, args.retry_delay, "Non-destructive detach route probe. Chrome should remain open; the script immediately restores attach.", "{}")
            idx += 1
            request_with_logging(log_path, stats, idx, "DETACH", "GET", root_base_url, "/admin/status", api_key, args.timeout, args.retries, args.retry_delay, "Admin status immediately after detach.", None)
            idx += 1
            request_with_logging(log_path, stats, idx, "DETACH", "GET", root_base_url, "/v1/browser/connection-info", api_key, args.timeout, args.retries, args.retry_delay, "Browser connection info immediately after detach.", None)
            idx += 1
            request_with_logging(log_path, stats, idx, "REATTACH", "POST", root_base_url, "/v1/browser/attach", api_key, args.timeout, args.retries, args.retry_delay, "Restore attached state after detach probe. The following convergence check must pass.", "{}")
            idx += 1

            idx, converged, failures, _observed = wait_for_attach_convergence(
                log_path=log_path,
                stats=stats,
                next_index=idx,
                label="reattach after detach",
                root_base_url=root_base_url,
                api_key=api_key,
                timeout_seconds=args.timeout,
                retries=args.retries,
                retry_delay_seconds=args.retry_delay,
                convergence_timeout_seconds=args.attach_convergence_timeout,
                convergence_interval_seconds=args.attach_convergence_interval,
            )
            if not converged:
                stats.semantic_failures.extend([f"reattach convergence failed: {failure}" for failure in failures])
        else:
            append_log(log_path, "\nDetach/reattach cycle skipped by `--skip-detach-cycle`.\n")

        append_log(log_path, "\n# E. Metadata and provider route sweep\n\n")
        metadata = [
            ("GET", "/v1/models", "Provider model metadata.", None),
            ("GET", "/v1/capabilities", "Provider/gateway capabilities.", None),
            ("GET", "/v1/provider", "Provider identity/status metadata.", None),
        ]
        for method, path, note, body in metadata:
            request_with_logging(log_path, stats, idx, "META", method, root_base_url, path, api_key, args.timeout, args.retries, args.retry_delay, note, body)
            idx += 1

        append_log(log_path, "\n# F. State discovery route sweep\n\n")
        discovery_results: Dict[str, Dict[str, Any]] = {}
        discovery = [
            ("GET", "/v1/jobs", "Discover jobs.", None, "jobs"),
            ("GET", "/v1/sessions", "Discover sessions.", None, "sessions"),
            ("GET", "/v1/requests", "Discover requests.", None, "requests"),
            ("GET", "/v1/usage", "Discover usage state.", None, "usage"),
            ("GET", "/v1/conversations", "Discover conversations.", None, "conversations"),
            ("GET", "/v1/files", "Discover staged files.", None, "files"),
            ("GET", "/v1/artifacts", "Discover artifacts.", None, "artifacts"),
        ]
        for method, path, note, body, key in discovery:
            result = request_with_logging(log_path, stats, idx, "DISCOVERY", method, root_base_url, path, api_key, args.timeout, args.retries, args.retry_delay, note, body)
            discovery_results[key] = result
            idx += 1

        dynamic_ids: Dict[str, List[str]] = {}
        dynamic_ids["jobs"] = collect_string_ids(discovery_results.get("jobs", {}).get("json"), ["job_id", "id"])
        dynamic_ids["sessions"] = collect_string_ids(discovery_results.get("sessions", {}).get("json"), ["session_id", "id"])
        dynamic_ids["requests"] = collect_string_ids(discovery_results.get("requests", {}).get("json"), ["request_id", "id"])
        dynamic_ids["conversations"] = collect_string_ids(discovery_results.get("conversations", {}).get("json"), ["conversation_id", "id"])
        dynamic_ids["files"] = collect_string_ids(discovery_results.get("files", {}).get("json"), ["file_id", "id"])
        dynamic_ids["artifacts"] = collect_string_ids(discovery_results.get("artifacts", {}).get("json"), ["artifact_id", "id"])

        append_log(log_path, "\n## Extracted dynamic IDs for follow-up reads\n\n")
        append_log(log_path, fence("json", json.dumps(dynamic_ids, indent=2, sort_keys=True)))

        append_log(log_path, "\n# G. Dynamic read routes with known-invalid IDs\n\n")
        invalid_id = "step2-known-invalid-id"
        dynamic_invalid = [
            ("GET", f"/v1/jobs/{id_segment(invalid_id)}", "Known-invalid job ID.", None),
            ("GET", f"/v1/sessions/{id_segment(invalid_id)}/conversations", "Known-invalid session ID conversations.", None),
            ("GET", f"/v1/requests/{id_segment(invalid_id)}", "Known-invalid request ID.", None),
            ("GET", f"/v1/requests/{id_segment(invalid_id)}/evidence", "Known-invalid request evidence.", None),
            ("GET", f"/v1/conversations/{id_segment(invalid_id)}", "Known-invalid conversation ID.", None),
            ("GET", f"/v1/conversations/{id_segment(invalid_id)}/messages", "Known-invalid conversation messages.", None),
            ("GET", f"/v1/conversations/{id_segment(invalid_id)}/snapshot", "Known-invalid conversation snapshot.", None),
            ("GET", f"/v1/files/{id_segment(invalid_id)}", "Known-invalid file ID.", None),
            ("GET", f"/v1/artifacts/{id_segment(invalid_id)}", "Known-invalid artifact ID.", None),
        ]
        for method, path, note, body in dynamic_invalid:
            request_with_logging(log_path, stats, idx, "DYNAMIC_INVALID", method, root_base_url, path, api_key, args.timeout, args.retries, args.retry_delay, note, body)
            idx += 1

        append_log(log_path, "\n# H. Dynamic read routes with discovered IDs, when available\n\n")
        real_dynamic: List[Tuple[str, str, str, Optional[str]]] = []
        if dynamic_ids["jobs"]:
            real_dynamic.append(("GET", f"/v1/jobs/{id_segment(dynamic_ids['jobs'][0])}", "Discovered job ID.", None))
        if dynamic_ids["sessions"]:
            real_dynamic.append(("GET", f"/v1/sessions/{id_segment(dynamic_ids['sessions'][0])}/conversations", "Discovered session ID conversations.", None))
        if dynamic_ids["requests"]:
            real_dynamic.append(("GET", f"/v1/requests/{id_segment(dynamic_ids['requests'][0])}", "Discovered request ID.", None))
            real_dynamic.append(("GET", f"/v1/requests/{id_segment(dynamic_ids['requests'][0])}/evidence", "Discovered request evidence.", None))
        if dynamic_ids["conversations"]:
            real_dynamic.append(("GET", f"/v1/conversations/{id_segment(dynamic_ids['conversations'][0])}", "Discovered conversation ID.", None))
            real_dynamic.append(("GET", f"/v1/conversations/{id_segment(dynamic_ids['conversations'][0])}/messages", "Discovered conversation messages.", None))
            real_dynamic.append(("GET", f"/v1/conversations/{id_segment(dynamic_ids['conversations'][0])}/snapshot", "Discovered conversation snapshot.", None))
        if dynamic_ids["files"]:
            real_dynamic.append(("GET", f"/v1/files/{id_segment(dynamic_ids['files'][0])}", "Discovered file ID.", None))
        if dynamic_ids["artifacts"]:
            real_dynamic.append(("GET", f"/v1/artifacts/{id_segment(dynamic_ids['artifacts'][0])}", "Discovered artifact ID.", None))

        if real_dynamic:
            for method, path, note, body in real_dynamic:
                request_with_logging(log_path, stats, idx, "DYNAMIC_REAL", method, root_base_url, path, api_key, args.timeout, args.retries, args.retry_delay, note, body)
                idx += 1
        else:
            append_log(log_path, "No discovered IDs were found in discovery responses, so real dynamic reads were skipped.\n")

        append_log(log_path, "\n# I. Auth and route-contract probes\n\n")
        contract_probes = [
            ("GET", "/v1/status", "Missing bearer token against authenticated v1 route.", None, "missing", None),
            ("GET", "/v1/status", "Wrong bearer token against authenticated v1 route.", None, "wrong", None),
            ("GET", "/v1/status", "Malformed authorization header against authenticated v1 route.", None, "malformed", None),
            ("POST", "/health", "Wrong method against /health.", "{}", "valid", None),
            ("PUT", "/ready", "Wrong method against /ready.", "{}", "valid", None),
            ("DELETE", "/v1/models", "Wrong method against /v1/models.", None, "valid", None),
            ("PATCH", "/v1/status", "Wrong method against /v1/status.", "{}", "valid", None),
            ("GET", "/v1/definitely-not-a-real-step2-route", "Unknown v1 route.", None, "valid", None),
            ("GET", "/v1/requests?limit=not-an-int&offset=-1", "Bad query values for requests discovery.", None, "valid", None),
            ("GET", "/v1/files?limit=-1&offset=not-an-int", "Bad query values for files discovery.", None, "valid", None),
            ("POST", "/v1/browser/attach", "Malformed JSON body against non-prompting attach route.", "{ definitely malformed json", "valid", "application/json"),
            ("POST", "/v1/browser/attach", "Wrong content type body against non-prompting attach route.", "plain text body", "valid", "text/plain"),
        ]
        for method, path, note, body, auth_mode, content_type in contract_probes:
            request_with_logging(
                log_path,
                stats,
                idx,
                "CONTRACT",
                method,
                root_base_url,
                path,
                api_key,
                args.timeout,
                args.retries,
                args.retry_delay,
                note,
                body,
                auth_mode=auth_mode,
                content_type=content_type,
            )
            idx += 1

        append_log(log_path, "\n# J. Restore attach after contract probes and require convergence\n\n")
        request_with_logging(log_path, stats, idx, "RESTORE", "POST", root_base_url, "/v1/browser/attach", api_key, args.timeout, args.retries, args.retry_delay, "Restore attach after route-contract probes. The following convergence check must pass.", "{}")
        idx += 1
        idx, converged, failures, _observed = wait_for_attach_convergence(
            log_path=log_path,
            stats=stats,
            next_index=idx,
            label="final restore attach",
            root_base_url=root_base_url,
            api_key=api_key,
            timeout_seconds=args.timeout,
            retries=args.retries,
            retry_delay_seconds=args.retry_delay,
            convergence_timeout_seconds=args.attach_convergence_timeout,
            convergence_interval_seconds=args.attach_convergence_interval,
        )
        if not converged:
            stats.semantic_failures.extend([f"final restore attach convergence failed: {failure}" for failure in failures])

        append_log(log_path, "\n# K. Focused stability loops after final convergence attempt\n\n")
        noisy_paths = ["/v1/config", "/ready", "/health", "/admin/status", "/v1/status", "/v1/browser/connection-info", "/v1/browser/profile-info"]
        for loop_no in range(1, args.stability_loops + 1):
            append_log(log_path, f"\n## Stability loop {loop_no}\n\n")
            progress("STABILITY", f"loop {loop_no}/{args.stability_loops}")
            for path in noisy_paths:
                result = request_with_logging(
                    log_path,
                    stats,
                    idx,
                    "STABILITY",
                    "GET",
                    root_base_url,
                    path,
                    api_key,
                    args.timeout,
                    args.retries,
                    args.retry_delay,
                    f"Repeated stability check for {path}.",
                )
                if path in ("/health", "/admin/status", "/v1/browser/connection-info", "/v1/browser/profile-info") and int(result.get("status_code", 0) or 0) == 200:
                    ok, _fields, failures = endpoint_attached_state(path, result.get("json"))
                    if not ok:
                        stats.semantic_failures.extend([f"stability loop {loop_no}: {failure}" for failure in failures])
                idx += 1
                time.sleep(0.2)

        if stats.transport_final_failures > 0:
            problems.append("one or more routes still had final transport failure after retries")
        if stats.transport_first_attempt_failures > 0:
            warnings.append("one or more routes had first-attempt transport failures that may have recovered on retry")
        if stats.semantic_failures:
            problems.extend(stats.semantic_failures)
        if stats.semantic_warnings:
            warnings.extend(stats.semantic_warnings)

        if problems:
            verdict = "failed"
        elif warnings:
            verdict = "healthy_with_warnings"
        else:
            verdict = "healthy"

    except KeyboardInterrupt:
        verdict = "interrupted"
        problems.append("runner interrupted")
        append_log(log_path, "\n# Runner interrupted\n\n")
    except Exception as exc:
        verdict = "runner_failed"
        problems.append(f"{type(exc).__name__}: {exc}")
        append_log(log_path, "\n# Runner exception\n\n")
        append_log(log_path, fence("text", traceback.format_exc()))
    finally:
        progress("SUMMARY", f"verdict={verdict}")
        progress("SUMMARY", f"log file: {log_path}")
        for problem in problems[:20]:
            progress("PROBLEM", problem)
        if len(problems) > 20:
            progress("PROBLEM", f"{len(problems) - 20} additional problems omitted from console; see markdown log")
        for warning in warnings:
            progress("WARNING", warning)

        append_log(log_path, "\n# Final step 2 verdict\n\n")
        append_log(log_path, f"Finished UTC: `{utc_now_iso()}`  \n")
        append_log(log_path, f"Verdict: `{verdict}`\n\n")
        append_log(log_path, "## Summary data\n\n")
        append_log(log_path, fence("json", json.dumps(stats.as_dict(), indent=2, sort_keys=True)))
        append_log(log_path, "\n## Warnings\n\n")
        if warnings:
            for warning in warnings:
                append_log(log_path, f"- {warning}\n")
        else:
            append_log(log_path, "- none\n")
        append_log(log_path, "\n## Problems\n\n")
        if problems:
            for problem in problems:
                append_log(log_path, f"- {problem}\n")
        else:
            append_log(log_path, "- none\n")

    if verdict == "healthy":
        return 0
    if verdict == "healthy_with_warnings" and args.exit_zero_on_warnings:
        return 0
    if verdict == "healthy_with_warnings":
        return 2
    if verdict == "interrupted":
        return 130
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
