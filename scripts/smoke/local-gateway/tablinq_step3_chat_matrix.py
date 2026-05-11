#!/usr/bin/env python3
from __future__ import annotations

import argparse
import datetime as dt
import json
import os
import subprocess
import sys
import time
import traceback
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path
from typing import Any, Dict, Iterable, List, Optional, Sequence, Tuple

API_KEY_ENV_NAMES = ("TABLINQ_GATEWAY_API_KEY", "TABLINQ_API_KEY", "GATEWAY_API_KEY", "OPENAI_API_KEY", "API_KEY")
BASE_URL_ENV_NAMES = ("TABLINQ_GATEWAY_BASE_URL", "GATEWAY_BASE_URL", "OPENAI_BASE_URL", "BASE_URL")
DEFAULT_ROOT_BASE_URL = "http://127.0.0.1:3847"
CONVERGENCE_ENDPOINTS = ["/health", "/admin/status", "/v1/browser/profile-info", "/v1/browser/connection-info"]


class RunState:
    def __init__(self) -> None:
        self.total_requests = 0
        self.http_status_counts: Dict[str, int] = {}
        self.transport_first_attempt_failures = 0
        self.transport_final_failures = 0
        self.retry_recovered = 0
        self.routes_with_transport_warnings: List[str] = []
        self.cli_failures: List[str] = []
        self.semantic_failures: List[str] = []
        self.warnings: List[str] = []
        self.valid_chat_cases = 0
        self.valid_chat_2xx = 0
        self.chat_valid_non_2xx = 0
        self.chat_post_transport_ambiguous = 0
        self.ids: Dict[str, str] = {}

    def record_http(self, label: str, result: Dict[str, Any]) -> None:
        self.total_requests += 1
        status = str(result.get("status_code", 0))
        self.http_status_counts[status] = self.http_status_counts.get(status, 0) + 1
        attempts = result.get("attempts") or [result]
        first_status = int(attempts[0].get("status_code", 0) or 0)
        final_status = int(result.get("status_code", 0) or 0)
        if first_status == 0:
            self.transport_first_attempt_failures += 1
            self.routes_with_transport_warnings.append(label)
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
            "cli_failures": self.cli_failures,
            "semantic_failures": self.semantic_failures,
            "warnings": self.warnings,
            "valid_chat_cases": self.valid_chat_cases,
            "valid_chat_2xx": self.valid_chat_2xx,
            "chat_valid_non_2xx": self.chat_valid_non_2xx,
            "chat_post_transport_ambiguous": self.chat_post_transport_ambiguous,
            "ids": self.ids,
        }


def now_local() -> str:
    return dt.datetime.now().strftime("%H:%M:%S")


def utc_now() -> str:
    return dt.datetime.now(dt.timezone.utc).isoformat(timespec="seconds")


def stamp() -> str:
    return dt.datetime.now().strftime("%Y%m%d-%H%M%S")


def run_id_stamp() -> str:
    return dt.datetime.now().strftime("step3-v5-%Y%m%d-%H%M%S")


def progress(phase: str, message: str) -> None:
    print(f"[{now_local()}] {phase}: {message}", flush=True)


def append_log(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("a", encoding="utf-8", newline="\n") as f:
        f.write(text)


def fence(label: str, body: Any) -> str:
    text = "" if body is None else str(body)
    text = text.replace("\r\n", "\n").replace("\r", "\n")
    if text and not text.endswith("\n"):
        text += "\n"
    return f"~~~{label}\n{text}~~~\n"


def parse_env_file(path: Path) -> Dict[str, str]:
    values: Dict[str, str] = {}
    if not path.exists():
        return values
    try:
        lines = path.read_text(encoding="utf-8-sig").splitlines()
    except UnicodeDecodeError:
        lines = path.read_text(encoding="utf-8", errors="replace").splitlines()
    for raw in lines:
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        if line.lower().startswith("export "):
            line = line[7:].strip()
        if "=" not in line:
            continue
        key, value = line.split("=", 1)
        key, value = key.strip(), value.strip()
        if not key:
            continue
        if len(value) >= 2 and ((value[0] == '"' and value[-1] == '"') or (value[0] == "'" and value[-1] == "'")):
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


def redact(text: str, api_key: Optional[str]) -> str:
    return text.replace(api_key, "<REDACTED_API_KEY>") if api_key else text


def quote_for_curl(value: str) -> str:
    return '"' + value.replace("\\", "\\\\").replace('"', '\\"') + '"'


def build_curl(method: str, url: str, headers: Dict[str, str], body_bytes: Optional[bytes]) -> str:
    parts = ["curl.exe", "-i", "-sS", "-X", method.upper(), quote_for_curl(url)]
    for key, value in headers.items():
        parts.extend(["-H", quote_for_curl(f"{key}: {value}")])
    if body_bytes is not None:
        parts.extend(["--data-raw", quote_for_curl(body_bytes.decode("utf-8", errors="replace"))])
    return " ".join(parts)


def response_headers_to_text(status_code: int, reason: str, items: Iterable[Tuple[str, str]]) -> str:
    lines = [f"HTTP/1.1 {status_code} {reason}".rstrip()]
    for key, value in items:
        lines.append(f"{key}: {value}")
    return "\n".join(lines) + "\n"


def parse_json(body: str) -> Optional[Any]:
    try:
        return json.loads(body)
    except Exception:
        return None


def raw_http(method: str, url: str, headers: Dict[str, str], body_bytes: Optional[bytes], timeout: int) -> Dict[str, Any]:
    started = utc_now()
    status_code = 0
    reason = "TRANSPORT ERROR"
    response_headers = ""
    response_body = ""
    error_type = ""
    request = urllib.request.Request(url=url, data=body_bytes, headers=headers, method=method.upper())
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
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
        "finished": utc_now(),
        "status_code": status_code,
        "reason": reason,
        "headers": response_headers,
        "body": response_body,
        "json": parse_json(response_body),
        "transport_error_type": error_type,
    }


def absolute_url(root: str, path: str) -> str:
    if path.startswith("http://") or path.startswith("https://"):
        return path
    return root + (path if path.startswith("/") else "/" + path)


def request_with_logging(
    log_path: Path,
    state: RunState,
    index: int,
    section: str,
    method: str,
    root: str,
    path: str,
    api_key: str,
    timeout: int,
    retries: int,
    retry_delay: float,
    note: str,
    body_obj: Optional[Any] = None,
    body_text: Optional[str] = None,
    content_type: str = "application/json",
    no_transport_retry: bool = False,
    extra_headers: Optional[Dict[str, str]] = None,
    auth_mode: str = "valid",
) -> Dict[str, Any]:
    method = method.upper()
    url = absolute_url(root, path)
    if body_obj is not None and body_text is not None:
        raise ValueError("body_obj and body_text are mutually exclusive")
    if body_obj is not None:
        body_text = json.dumps(body_obj, ensure_ascii=False, separators=(",", ":"))
    body_bytes = body_text.encode("utf-8") if body_text is not None else None

    headers: Dict[str, str] = {"Accept": "application/json"}
    if auth_mode == "valid":
        headers["Authorization"] = f"Bearer {api_key}"
    elif auth_mode == "missing":
        pass
    elif auth_mode == "wrong":
        headers["Authorization"] = "Bearer definitely-wrong-step3-token"
    elif auth_mode == "malformed":
        headers["Authorization"] = "Bearer"
    else:
        raise ValueError(f"unknown auth_mode: {auth_mode}")

    if body_bytes is not None:
        headers["Content-Type"] = content_type
    if extra_headers:
        headers.update(extra_headers)

    curl_headers = dict(headers)
    if curl_headers.get("Authorization") == f"Bearer {api_key}":
        curl_headers["Authorization"] = "Bearer <REDACTED_API_KEY>"

    effective_retries = 0 if no_transport_retry else retries
    curl = build_curl(method, url, curl_headers, body_bytes)

    append_log(log_path, f"\n## {section} {index:03d}: `{method} {path}`\n\n")
    append_log(log_path, f"Note: {note}\n\n")
    append_log(log_path, f"Auth mode: `{auth_mode}`  \nRetries for transport errors: `{effective_retries}`  \nNo transport retry: `{str(no_transport_retry).lower()}`\n\n")
    append_log(log_path, "### cURL\n\n" + fence("bash", redact(curl, api_key)))

    attempts: List[Dict[str, Any]] = []
    for attempt in range(1, effective_retries + 2):
        progress(section, f"{index:03d} attempt {attempt}/{effective_retries + 1}: {method} {path}")
        result = raw_http(method, url, headers, body_bytes, timeout)
        attempts.append(result)
        append_log(log_path, f"\n### Attempt {attempt}\n\n")
        append_log(log_path, f"Started UTC: `{result['started']}`  \nFinished UTC: `{result['finished']}`  \nStatus seen by smoke runner: `{result['status_code']}`  \nTransport error type: `{result['transport_error_type']}`\n\n")
        append_log(log_path, "#### Raw response headers\n\n" + fence("http", redact(result["headers"], api_key)))
        append_log(log_path, "\n#### Raw response body\n\n" + fence("text", redact(result["body"], api_key)))
        if int(result["status_code"]) != 0:
            break
        if attempt <= effective_retries:
            append_log(log_path, f"\nTransport error retry delay seconds: `{retry_delay}`\n")
            progress(section, f"{index:03d} transport error; retrying after {retry_delay}s")
            time.sleep(retry_delay)

    final = attempts[-1]
    final["attempts"] = attempts
    final["attempt_count"] = len(attempts)
    final["retried"] = len(attempts) > 1
    state.record_http(f"{section} {index:03d} {method} {path}", final)
    return final


def run_cli(log_path: Path, state: RunState, index: int, command: Sequence[str], timeout: int, api_key: Optional[str], note: str, allow_nonzero: bool = False) -> Dict[str, Any]:
    rendered = " ".join(command)
    progress("CLI", f"{index:03d}: {rendered}")
    started = utc_now()
    stdout = ""
    stderr = ""
    exit_code = 999
    timed_out = False
    try:
        completed = subprocess.run(list(command), text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=timeout, shell=False)
        stdout = completed.stdout or ""
        stderr = completed.stderr or ""
        exit_code = int(completed.returncode)
    except FileNotFoundError as exc:
        stderr = f"{type(exc).__name__}: {exc}"
        exit_code = 127
    except subprocess.TimeoutExpired as exc:
        timed_out = True
        exit_code = 124
        stdout = exc.stdout or ""
        stderr = exc.stderr or ""
        if isinstance(stdout, bytes):
            stdout = stdout.decode("utf-8", errors="replace")
        if isinstance(stderr, bytes):
            stderr = stderr.decode("utf-8", errors="replace")
        stderr += f"\nTimeoutExpired: command exceeded {timeout} seconds"
    except Exception as exc:
        stderr = f"{type(exc).__name__}: {exc}"
        exit_code = 1

    append_log(log_path, f"\n## CLI {index:03d}: `{redact(rendered, api_key)}`\n\n")
    append_log(log_path, f"Note: {note}\n\nStarted UTC: `{started}`  \nFinished UTC: `{utc_now()}`  \nExit code: `{exit_code}`  \nTimed out: `{str(timed_out).lower()}`\n\n")
    append_log(log_path, "### Command\n\n" + fence("text", "> " + redact(rendered, api_key)))
    append_log(log_path, "\n### stdout\n\n" + fence("text", redact(stdout, api_key)))
    append_log(log_path, "\n### stderr\n\n" + fence("text", redact(stderr, api_key)))
    if exit_code != 0 and not allow_nonzero:
        state.cli_failures.append(f"{rendered} exited {exit_code}")
    return {"exit_code": exit_code, "stdout": stdout, "stderr": stderr, "timed_out": timed_out}


def get_path(value: Any, path: str) -> Any:
    node = value
    for part in path.split("."):
        if isinstance(node, dict) and part in node:
            node = node[part]
        else:
            return None
    return node


def endpoint_attached_state(path: str, body: Any) -> Tuple[bool, Dict[str, Any], List[str]]:
    if not isinstance(body, dict):
        return False, {"json_object": False}, [f"{path}: response body is not a JSON object"]
    if path == "/health":
        expected = {"status": "ok", "health.status": "ok", "health.authenticated": True, "health.browser_connected": True, "health.extension_connected": True}
    elif path == "/admin/status":
        expected = {"health.status": "ok", "health.authenticated": True, "health.browser_connected": True, "health.extension_connected": True, "browser.authenticated": True, "browser.cdp_attached": True, "browser.extension_connected": True, "browser.page_ready": True, "browser.running": True, "browser.target_tab_found": True}
    elif path == "/v1/browser/profile-info":
        expected = {"data.auth_source": "page", "data.authenticated": True, "data.cdp_attached": True, "data.extension_connected": True, "data.page_ready": True, "data.running": True, "data.target_tab_found": True}
    elif path == "/v1/browser/connection-info":
        expected = {"browser.auth_source": "page", "browser.authenticated": True, "browser.cdp_attached": True, "browser.extension_connected": True, "browser.page_ready": True, "browser.running": True, "browser.target_tab_found": True}
    else:
        return False, {"unknown_endpoint": path}, [f"{path}: unknown convergence endpoint"]

    fields: Dict[str, Any] = {}
    failures: List[str] = []
    for key, expected_value in expected.items():
        actual = get_path(body, key)
        fields[key] = actual
        if actual != expected_value:
            failures.append(f"{path}: expected {key}={expected_value!r}, got {actual!r}")
    return len(failures) == 0, fields, failures


def api_attach_result_healthy(result: Dict[str, Any]) -> bool:
    body = result.get("json")
    data = body.get("data") if isinstance(body, dict) else None
    return isinstance(data, dict) and data.get("auth_source") == "page" and data.get("authenticated") is True and data.get("cdp_attached") is True and data.get("extension_connected") is True and data.get("page_ready") is True and data.get("running") is True and data.get("target_tab_found") is True


def wait_convergence(log_path: Path, state: RunState, idx: int, root: str, api_key: str, timeout: int, retries: int, retry_delay: float, convergence_timeout: int, interval: float, label: str) -> Tuple[int, bool, List[str]]:
    append_log(log_path, f"\n# Attach convergence check: {label}\n\n")
    deadline = time.monotonic() + convergence_timeout
    round_no = 0
    last_failures: List[str] = []
    while time.monotonic() < deadline:
        round_no += 1
        append_log(log_path, f"\n## Convergence round {round_no}: {label}\n\n")
        progress("CONVERGE", f"{label}: round {round_no}")
        observed: Dict[str, Any] = {}
        failures: List[str] = []
        all_ok = True
        for endpoint in CONVERGENCE_ENDPOINTS:
            result = request_with_logging(log_path, state, idx, "CONVERGE", "GET", root, endpoint, api_key, timeout, retries, retry_delay, f"{label}: semantic convergence check for {endpoint}.")
            idx += 1
            if int(result.get("status_code", 0) or 0) != 200:
                all_ok = False
                failure = f"{endpoint}: expected HTTP 200 during convergence, got {result.get('status_code')}"
                failures.append(failure)
                observed[endpoint] = {"status_code": result.get("status_code"), "ok": False, "fields": None}
                continue
            ok, fields, endpoint_failures = endpoint_attached_state(endpoint, result.get("json"))
            observed[endpoint] = {"status_code": result.get("status_code"), "ok": ok, "fields": fields}
            if not ok:
                all_ok = False
                failures.extend(endpoint_failures)
        append_log(log_path, "\n### Extracted convergence fields\n\n" + fence("json", json.dumps(observed, indent=2, sort_keys=True)))
        append_log(log_path, "\n### Convergence failures this round\n\n" + ("".join(f"- {failure}\n" for failure in failures) if failures else "- none\n"))
        last_failures = failures
        if all_ok:
            append_log(log_path, f"\nConvergence result for `{label}`: `ok` after `{round_no}` round(s).\n")
            return idx, True, []
        time.sleep(interval)
    append_log(log_path, f"\nConvergence result for `{label}`: `failed` after `{round_no}` round(s).\n")
    return idx, False, last_failures


def collect_string_ids(value: Any, keys: Iterable[str]) -> List[str]:
    key_set = set(keys)
    found: List[str] = []
    def walk(node: Any) -> None:
        if isinstance(node, dict):
            for key, child in node.items():
                if key in key_set and isinstance(child, str) and child.strip():
                    found.append(child.strip())
                walk(child)
        elif isinstance(node, list):
            for child in node:
                walk(child)
    walk(value)
    out: List[str] = []
    seen = set()
    for item in found:
        if item not in seen:
            seen.add(item)
            out.append(item)
    return out


def extract_ids(result: Dict[str, Any]) -> Dict[str, str]:
    body = result.get("json")
    mapping = {
        "request_id": ["request_id", "tablinq_request_id"],
        "conversation_id": ["conversation_id", "tablinq_conversation_id"],
        "session_id": ["session_id", "tablinq_session_id"],
        "job_id": ["job_id", "tablinq_job_id"],
        "id": ["id"],
    }
    out: Dict[str, str] = {}
    for canonical, keys in mapping.items():
        values = collect_string_ids(body, keys)
        if values:
            out[canonical] = values[0]
    return out


def id_segment(value: str) -> str:
    return urllib.parse.quote(value, safe="")


def update_ids(state: RunState, prefix: str, ids: Dict[str, str]) -> None:
    for key, value in ids.items():
        state.ids[f"{prefix}.{key}"] = value


def choose_id(state: RunState, prefix: str, key: str) -> Optional[str]:
    return state.ids.get(f"{prefix}.{key}")


def request_id_for_case(run_id: str, case_id: str) -> str:
    safe = "".join(ch if ch.isalnum() or ch in "-_" else "-" for ch in case_id.lower())
    return f"{run_id}-{safe}"


def valid_cases() -> List[Dict[str, Any]]:
    return [
        {
            "case_id": "packing-cube-new",
            "note": "Instant new conversation.",
            "body": {
                "model": "tablinq-instant",
                "stream": False,
                "tablinq_new_conversation": True,
                "messages": [{"role": "user", "content": "A friend overpacks for every weekend trip. Give one practical thing to remove before blaming the suitcase."}],
            },
            "save_as": "packing_cube",
        },
        {
            "case_id": "packing-cube-followup",
            "note": "Instant follow-up using captured conversation id.",
            "requires_conversation": "packing_cube",
            "body": {
                "model": "tablinq-instant",
                "stream": False,
                "messages": [{"role": "user", "content": "Make that advice sound like a tiny note taped inside a carry-on."}],
            },
            "save_as": "packing_cube_followup",
        },
        {
            "case_id": "thinking-standard-layover",
            "note": "Thinking standard.",
            "body": {
                "model": "tablinq-thinking-standard",
                "stream": False,
                "tablinq_new_conversation": True,
                "messages": [{"role": "user", "content": "Someone can choose a short layover or a long layover. What tradeoff should a calm traveler notice?"}],
            },
            "save_as": "thinking_standard",
        },
        {
            "case_id": "search-flight-terms",
            "note": "Search mode without research.",
            "body": {
                "model": "tablinq-search",
                "stream": False,
                "tablinq_new_conversation": True,
                "messages": [{"role": "user", "content": "In two sentences, explain the difference between a nonstop flight and a direct flight."}],
            },
            "save_as": "search",
        },
        {
            "case_id": "thinking-extended-train",
            "note": "Thinking extended.",
            "body": {
                "model": "tablinq-thinking-extended",
                "stream": False,
                "tablinq_new_conversation": True,
                "messages": [{"role": "user", "content": "Imagine someone planning a train trip with one transfer. Name two small risks they might underestimate."}],
            },
            "save_as": "thinking_extended",
        },
        {
            "case_id": "multipart-hostel-note",
            "note": "Multipart chat content using OpenAI chat text parts only.",
            "body": {
                "model": "tablinq-instant",
                "stream": False,
                "tablinq_new_conversation": True,
                "messages": [{"role": "user", "content": [{"type": "text", "text": "I found a hostel listing that says quiet hours start at 10."}, {"type": "text", "text": "Give one polite question to ask before booking."}]}],
            },
            "save_as": "multipart",
        },
        {
            "case_id": "primary-travel-status",
            "note": "Primary role annotation.",
            "body": {
                "model": "tablinq-instant",
                "stream": False,
                "tablinq_new_conversation": True,
                "tablinq_request_role": "primary",
                "messages": [{"role": "user", "content": "Write a one-line trip note for a route that was scenic but had confusing signs."}],
            },
            "save_as": "primary",
        },
        {
            "case_id": "branch-travel-status",
            "note": "Branch role annotation using parent request id when available.",
            "requires_parent": "primary",
            "body": {
                "model": "tablinq-instant",
                "stream": False,
                "tablinq_new_conversation": True,
                "tablinq_request_role": "branch",
                "messages": [{"role": "user", "content": "Make the same trip note sound a little more cautious, still one line."}],
            },
            "save_as": "branch",
        },
        {
            "case_id": "auto-recover-travel",
            "note": "Auto-recover annotation. Expected normal success without recovery.",
            "expect_no_recovery": True,
            "body": {
                "model": "tablinq-instant",
                "stream": False,
                "tablinq_new_conversation": True,
                "tablinq_auto_recover_provider_generation_failure": True,
                "messages": [{"role": "user", "content": "A traveler keeps missing small details on printed tickets. What is one harmless first habit to try? One sentence."}],
            },
            "save_as": "auto_recover",
        },
    ]


def invalid_cases() -> List[Dict[str, Any]]:
    return [
        {"case_id": "unsupported-model", "note": "Unsupported model should fail before browser submission.", "body": {"model": "tablinq-made-up-model", "stream": False, "messages": [{"role": "user", "content": "Name one thing a traveler should check before leaving home."}]}},
        {"case_id": "missing-model", "note": "Missing model validation.", "body": {"stream": False, "messages": [{"role": "user", "content": "What makes a luggage tag useful?"}]}},
        {"case_id": "missing-messages", "note": "Missing messages validation.", "body": {"model": "tablinq-instant", "stream": False}},
        {"case_id": "empty-messages", "note": "Empty messages validation.", "body": {"model": "tablinq-instant", "stream": False, "messages": []}},
        {"case_id": "invalid-role", "note": "Unsupported message role validation.", "body": {"model": "tablinq-instant", "stream": False, "messages": [{"role": "tool", "content": "This should not be submitted."}]}},
        {"case_id": "malformed-json", "note": "Malformed JSON validation.", "raw_body": "{\"model\":\"tablinq-instant\",\"messages\":[", "content_type": "application/json"},
        {"case_id": "wrong-content-type", "note": "Wrong content type validation.", "raw_body": "model=tablinq-instant&messages=not-json", "content_type": "text/plain"},
    ]


def find_request_item(body: Any, request_id: str) -> Optional[Dict[str, Any]]:
    if not isinstance(body, dict):
        return None
    data = body.get("data")
    if not isinstance(data, list):
        return None
    for item in data:
        if isinstance(item, dict) and item.get("id") == request_id:
            return item
    return None


def assert_no_recovery_contract(state: RunState, case_id: str, chat_result: Dict[str, Any], request_detail: Optional[Dict[str, Any]], requests_list: Optional[Dict[str, Any]], request_id: Optional[str]) -> None:
    failures: List[str] = []
    chat_json = chat_result.get("json")
    if isinstance(chat_json, dict):
        if chat_json.get("tablinq_recovery_attempted") not in (False, None):
            failures.append(f"{case_id}: chat response reported tablinq_recovery_attempted={chat_json.get('tablinq_recovery_attempted')!r}; expected false/null")
        if chat_json.get("tablinq_recovery_available") not in (False, None):
            failures.append(f"{case_id}: chat response reported tablinq_recovery_available={chat_json.get('tablinq_recovery_available')!r}; expected false/null")
        for key in ("tablinq_recovery_strategy", "tablinq_replacement_conversation_id", "tablinq_replacement_request_id"):
            if chat_json.get(key) is not None:
                failures.append(f"{case_id}: chat response {key} should be null when no recovery occurred")
    else:
        failures.append(f"{case_id}: chat response was not a JSON object; cannot validate no-recovery contract")

    detail_data = None
    if request_detail and isinstance(request_detail.get("json"), dict):
        detail_data = request_detail["json"].get("data")
    if isinstance(detail_data, dict):
        if detail_data.get("status") != "completed":
            failures.append(f"{case_id}: request detail status should be completed when no recovery occurred, got {detail_data.get('status')!r}")
        for key in ("recovered_at", "recovery_packet", "recovery_reason", "recovery_trace"):
            if detail_data.get(key) is not None:
                failures.append(f"{case_id}: request detail {key} should be null when no recovery occurred")
    else:
        failures.append(f"{case_id}: request detail data was unavailable; cannot validate no-recovery contract")

    if request_id and requests_list and isinstance(requests_list.get("json"), dict):
        item = find_request_item(requests_list["json"], request_id)
        if item is None:
            failures.append(f"{case_id}: request {request_id} was not found in /v1/requests")
        elif item.get("status") != "completed":
            failures.append(f"{case_id}: /v1/requests status should be completed when no recovery occurred, got {item.get('status')!r}")
    elif request_id:
        failures.append(f"{case_id}: /v1/requests response unavailable; cannot validate list status")

    if failures:
        state.semantic_failures.extend(failures)


def followup_reads(log_path: Path, state: RunState, idx: int, root: str, api_key: str, timeout: int, retries: int, retry_delay: float, inter_get_sleep: float, ids: Dict[str, str], label: str) -> Tuple[int, Dict[str, Dict[str, Any]]]:
    append_log(log_path, f"\n# Follow-up reads for {label}\n\n")
    results: Dict[str, Dict[str, Any]] = {}
    paths: List[Tuple[str, str, str]] = []
    request_id = ids.get("request_id")
    conversation_id = ids.get("conversation_id")
    session_id = ids.get("session_id")

    if request_id:
        paths.append(("request_detail", f"/v1/requests/{id_segment(request_id)}", "Request detail."))
        paths.append(("request_evidence", f"/v1/requests/{id_segment(request_id)}/evidence", "Request evidence."))
    paths.append(("usage", "/v1/usage", "Usage state."))
    if conversation_id:
        paths.append(("conversation", f"/v1/conversations/{id_segment(conversation_id)}", "Conversation detail."))
        paths.append(("messages", f"/v1/conversations/{id_segment(conversation_id)}/messages", "Conversation messages."))
        paths.append(("snapshot", f"/v1/conversations/{id_segment(conversation_id)}/snapshot", "Conversation snapshot."))
    if session_id:
        paths.append(("session_conversations", f"/v1/sessions/{id_segment(session_id)}/conversations", "Session conversations."))
    paths.extend([
        ("requests_list", "/v1/requests", "Requests discovery."),
        ("jobs", "/v1/jobs", "Jobs discovery."),
        ("admin_status", "/admin/status", "Admin/browser status."),
    ])

    for key, path, note in paths:
        results[key] = request_with_logging(log_path, state, idx, "FOLLOWUP", "GET", root, path, api_key, timeout, retries, retry_delay, f"{label}: {note}")
        idx += 1
        if inter_get_sleep > 0:
            progress("SLEEP", f"{label}: sleeping {inter_get_sleep}s between follow-up GETs")
            time.sleep(inter_get_sleep)
    return idx, results


def ambiguous_post_inspection(log_path: Path, state: RunState, idx: int, root: str, api_key: str, timeout: int, retries: int, retry_delay: float, inter_get_sleep: float, label: str) -> int:
    append_log(log_path, f"\n# Ambiguous prompt POST inspection for {label}\n\nA prompt-submitting POST had a transport failure. The runner will not resubmit the prompt.\n")
    for path in ["/v1/requests", "/v1/jobs", "/v1/conversations", "/v1/usage", "/admin/status", "/v1/doctor"]:
        request_with_logging(log_path, state, idx, "AMBIGUOUS_POST", "GET", root, path, api_key, timeout, retries, retry_delay, f"{label}: state inspection after ambiguous POST.")
        idx += 1
        if inter_get_sleep > 0:
            time.sleep(inter_get_sleep)
    return idx


def add_header(log_path: Path, env_file: Path, root: str, api_key_name: str, run_id: str, args: argparse.Namespace) -> None:
    append_log(log_path, "# Tablinq Gateway Step 3 Chat-Completions Smoke Log v5\n\n")
    append_log(log_path, f"Generated UTC: `{utc_now()}`  \nRun ID: `{run_id}`  \nRunner: `tablinq_step3_chat_matrix.py`  \nPython: `{sys.version.split()[0]}`  \n")
    append_log(log_path, f"Working directory: `{Path.cwd()}`  \nenv.local path: `{env_file}`  \nGateway root base URL: `{root}`  \nAPI key source: `{api_key_name}`  \nAPI key value: `<REDACTED_API_KEY>`\n\n")
    append_log(log_path, "Scope: step 3 chat-completions smoke. This run sends real non-research travel-themed prompts only after attach convergence. It never uses `tablinq-research` and never sends a deep research query.\n\n")
    append_log(log_path, "Safety: prompt POSTs are not blindly retried after transport errors. The auto-recover case asserts that a normal successful request remains `completed`, not `recovered`.\n\n")
    append_log(log_path, f"Between ask sleep seconds: `{args.between_ask_sleep}`  \nFollow-up delay seconds: `{args.followup_delay}`  \nInter-GET sleep seconds: `{args.inter_get_sleep}`  \nAfter ambiguous POST sleep seconds: `{args.after_ambiguous_post_sleep}`\n")


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Tablinq step 3 chat-completions smoke matrix v3.")
    parser.add_argument("--env-file", default=None)
    parser.add_argument("--base-url", default=None)
    parser.add_argument("--log-file", default=None)
    parser.add_argument("--gpt-bin", default=os.environ.get("TABLINQ_GPT_BIN", "gpt"))
    parser.add_argument("--chat-timeout", type=int, default=7200)
    parser.add_argument("--get-timeout", type=int, default=30)
    parser.add_argument("--cli-timeout", type=int, default=120)
    parser.add_argument("--get-retries", type=int, default=2)
    parser.add_argument("--retry-delay", type=float, default=0.75)
    parser.add_argument("--between-ask-sleep", type=float, default=14.0)
    parser.add_argument("--followup-delay", type=float, default=3.0)
    parser.add_argument("--inter-get-sleep", type=float, default=0.7)
    parser.add_argument("--after-ambiguous-post-sleep", type=float, default=20.0)
    parser.add_argument("--attach-convergence-timeout", type=int, default=25)
    parser.add_argument("--attach-convergence-interval", type=float, default=2.0)
    parser.add_argument("--skip-cli-attach", action="store_true")
    parser.add_argument("--include-invalid-cases", action="store_true")
    parser.add_argument("--exit-zero-on-warnings", action="store_true")
    return parser


def main(argv: Optional[Sequence[str]] = None) -> int:
    args = build_parser().parse_args(argv)
    script_dir = Path(__file__).resolve().parent
    env_file = Path(args.env_file).expanduser().resolve() if args.env_file else script_dir / ".env.local"
    env_values = parse_env_file(env_file)
    for key, value in env_values.items():
        os.environ.setdefault(key, value)

    api_key_name, api_key = first_present(API_KEY_ENV_NAMES, env_values)
    _base_name, env_base_url = first_present(BASE_URL_ENV_NAMES, env_values)
    root = normalize_root_base_url(args.base_url or env_base_url or DEFAULT_ROOT_BASE_URL)
    run_id = run_id_stamp()
    log_path = Path(args.log_file).expanduser().resolve() if args.log_file else script_dir / f"tablinq-step3-chat-matrix-v5-{stamp()}.md"
    if log_path.exists():
        log_path.unlink()

    state = RunState()
    problems: List[str] = []
    warnings: List[str] = []
    verdict = "runner_failed"
    idx = 1
    cli_idx = 1

    try:
        if not api_key:
            raise RuntimeError("No API key found. Expected one of: " + ", ".join(API_KEY_ENV_NAMES))

        progress("SETUP", f"log file: {log_path}")
        progress("SETUP", f"gateway URL: {root}")
        progress("SETUP", f"run id: {run_id}")
        add_header(log_path, env_file, root, api_key_name or "unknown", run_id, args)

        append_log(log_path, "\n# A. CLI preflight and attach\n\n")
        if not args.skip_cli_attach:
            run_cli(log_path, state, cli_idx, [args.gpt_bin, "status"], args.cli_timeout, api_key, "Status before CLI attach.", allow_nonzero=True); cli_idx += 1
            run_cli(log_path, state, cli_idx, [args.gpt_bin, "browser", "connection-info"], args.cli_timeout, api_key, "Browser connection-info before CLI attach.", allow_nonzero=True); cli_idx += 1
            run_cli(log_path, state, cli_idx, [args.gpt_bin, "browser", "attach"], args.cli_timeout, api_key, "CLI attach before API convergence.", allow_nonzero=False); cli_idx += 1
            run_cli(log_path, state, cli_idx, [args.gpt_bin, "status"], args.cli_timeout, api_key, "Status after CLI attach.", allow_nonzero=True); cli_idx += 1
        else:
            append_log(log_path, "CLI attach skipped by `--skip-cli-attach`.\n")

        append_log(log_path, "\n# B. API attach and convergence before prompt submission\n\n")
        attach_result = request_with_logging(log_path, state, idx, "ATTACH", "POST", root, "/v1/browser/attach", api_key, args.get_timeout, args.get_retries, args.retry_delay, "API attach before prompt submission.", body_obj={})
        idx += 1
        if api_attach_result_healthy(attach_result):
            append_log(log_path, "\nAPI attach response itself reported healthy attached browser state. Status endpoints must now converge before prompts are sent.\n")
        else:
            append_log(log_path, "\nAPI attach response did not itself report a fully healthy attached browser state.\n")

        idx, converged, failures = wait_convergence(log_path, state, idx, root, api_key, args.get_timeout, args.get_retries, args.retry_delay, args.attach_convergence_timeout, args.attach_convergence_interval, "after API attach")
        if not converged and api_attach_result_healthy(attach_result) and not args.skip_cli_attach:
            warning = "API attach returned healthy snapshot but status endpoints did not converge; running one CLI attach fallback"
            warnings.append(warning)
            state.warnings.append(warning)
            append_log(log_path, "\n# B2. CLI attach fallback after API/status inconsistency\n\n")
            run_cli(log_path, state, cli_idx, [args.gpt_bin, "browser", "attach"], args.cli_timeout, api_key, "CLI attach fallback after API/status inconsistency.", allow_nonzero=False); cli_idx += 1
            run_cli(log_path, state, cli_idx, [args.gpt_bin, "status"], args.cli_timeout, api_key, "Status after CLI fallback attach.", allow_nonzero=True); cli_idx += 1
            request_with_logging(log_path, state, idx, "ATTACH", "POST", root, "/v1/browser/attach", api_key, args.get_timeout, args.get_retries, args.retry_delay, "API attach after CLI fallback.", body_obj={})
            idx += 1
            idx, converged, failures = wait_convergence(log_path, state, idx, root, api_key, args.get_timeout, args.get_retries, args.retry_delay, args.attach_convergence_timeout, args.attach_convergence_interval, "after CLI fallback attach")

        if not converged:
            state.semantic_failures.extend([f"pre-prompt attach convergence failed: {failure}" for failure in failures])
            raise RuntimeError("pre-prompt attach convergence failed; refusing to send prompt-backed requests")

        append_log(log_path, "\n# C. Valid prompt-backed chat-completions cases\n\n")
        for case in valid_cases():
            case_id = case["case_id"]
            body = json.loads(json.dumps(case["body"]))

            if case.get("requires_conversation"):
                conversation_id = choose_id(state, case["requires_conversation"], "conversation_id")
                if not conversation_id:
                    warning = f"skipped {case_id}: missing conversation id from {case['requires_conversation']}"
                    warnings.append(warning)
                    state.warnings.append(warning)
                    append_log(log_path, f"\n## Skipped `{case_id}`\n\nReason: {warning}\n")
                    continue
                body["tablinq_conversation_id"] = conversation_id

            if case.get("requires_parent"):
                parent_id = choose_id(state, case["requires_parent"], "request_id")
                if not parent_id:
                    warning = f"skipped {case_id}: missing parent request id from {case['requires_parent']}"
                    warnings.append(warning)
                    state.warnings.append(warning)
                    append_log(log_path, f"\n## Skipped `{case_id}`\n\nReason: {warning}\n")
                    continue
                body["tablinq_parent_request_id"] = parent_id

            body.setdefault("metadata", {})
            body["metadata"]["step3_run_id"] = run_id
            body["metadata"]["step3_case_id"] = case_id
            body["metadata"]["prompt_theme"] = "travel"

            state.valid_chat_cases += 1
            result = request_with_logging(
                log_path, state, idx, "CHAT_VALID", "POST", root, "/v1/chat/completions", api_key,
                args.chat_timeout, 0, args.retry_delay, f"{case_id}: {case['note']} Non-research travel prompt.",
                body_obj=body, extra_headers={"X-Request-Id": request_id_for_case(run_id, case_id)}, no_transport_retry=True
            )
            idx += 1
            status = int(result.get("status_code", 0) or 0)

            if status == 0:
                state.chat_post_transport_ambiguous += 1
                warning = f"{case_id}: prompt POST transport failure; not retried"
                warnings.append(warning)
                state.warnings.append(warning)
                idx = ambiguous_post_inspection(log_path, state, idx, root, api_key, args.get_timeout, args.get_retries, args.retry_delay, args.inter_get_sleep, case_id)
                progress("SLEEP", f"{case_id}: sleeping {args.after_ambiguous_post_sleep}s after ambiguous prompt POST")
                time.sleep(args.after_ambiguous_post_sleep)
                continue

            ids = extract_ids(result)
            update_ids(state, case["save_as"], ids)
            append_log(log_path, f"\n### Extracted IDs for `{case_id}`\n\n" + fence("json", json.dumps(ids, indent=2, sort_keys=True)))

            if 200 <= status <= 299:
                state.valid_chat_2xx += 1
            else:
                state.chat_valid_non_2xx += 1
                warning = f"{case_id}: valid prompt case returned HTTP {status}"
                warnings.append(warning)
                state.warnings.append(warning)

            progress("SLEEP", f"{case_id}: sleeping {args.followup_delay}s before follow-up reads")
            time.sleep(args.followup_delay)
            idx, followups = followup_reads(log_path, state, idx, root, api_key, args.get_timeout, args.get_retries, args.retry_delay, args.inter_get_sleep, ids, case_id)

            if case.get("expect_no_recovery"):
                assert_no_recovery_contract(
                    state=state,
                    case_id=case_id,
                    chat_result=result,
                    request_detail=followups.get("request_detail"),
                    requests_list=followups.get("requests_list"),
                    request_id=ids.get("request_id"),
                )

            progress("SLEEP", f"{case_id}: sleeping {args.between_ask_sleep}s before next ask")
            time.sleep(args.between_ask_sleep)

        if args.include_invalid_cases:
            append_log(log_path, "\n# D. Invalid chat-completions validation cases\n\n")
            for case in invalid_cases():
                case_id = case["case_id"]
                if "raw_body" in case:
                    result = request_with_logging(
                        log_path, state, idx, "CHAT_INVALID", "POST", root, "/v1/chat/completions", api_key,
                        args.get_timeout, 0, args.retry_delay, f"{case_id}: {case['note']}",
                        body_text=case["raw_body"], content_type=case.get("content_type", "application/json"),
                        no_transport_retry=True, extra_headers={"X-Request-Id": request_id_for_case(run_id, case_id)}
                    )
                else:
                    body = json.loads(json.dumps(case["body"]))
                    body.setdefault("metadata", {})
                    body["metadata"]["step3_run_id"] = run_id
                    body["metadata"]["step3_case_id"] = case_id
                    body["metadata"]["prompt_theme"] = "travel"
                    result = request_with_logging(
                        log_path, state, idx, "CHAT_INVALID", "POST", root, "/v1/chat/completions", api_key,
                        args.get_timeout, 0, args.retry_delay, f"{case_id}: {case['note']}",
                        body_obj=body, no_transport_retry=True, extra_headers={"X-Request-Id": request_id_for_case(run_id, case_id)}
                    )
                idx += 1
                status = int(result.get("status_code", 0) or 0)
                if status == 0:
                    state.chat_post_transport_ambiguous += 1
                    warning = f"{case_id}: invalid chat POST transport failure; not retried"
                    warnings.append(warning)
                    state.warnings.append(warning)
                    idx = ambiguous_post_inspection(log_path, state, idx, root, api_key, args.get_timeout, args.get_retries, args.retry_delay, args.inter_get_sleep, case_id)
                    time.sleep(args.after_ambiguous_post_sleep)
                elif 200 <= status <= 299:
                    warning = f"{case_id}: invalid chat case returned HTTP {status}"
                    warnings.append(warning)
                    state.warnings.append(warning)
                time.sleep(2.0)
        else:
            append_log(log_path, "\nInvalid chat validation cases skipped by default. Use `-IncludeInvalidCases` to run them.\n")

        append_log(log_path, "\n# E. Final state inspection\n\n")
        for path in ["/health", "/admin/status", "/v1/status", "/v1/usage", "/v1/requests", "/v1/jobs", "/v1/conversations", "/v1/doctor"]:
            request_with_logging(log_path, state, idx, "FINAL", "GET", root, path, api_key, args.get_timeout, args.get_retries, args.retry_delay, f"Final state inspection for {path}.")
            idx += 1
            if args.inter_get_sleep > 0:
                time.sleep(args.inter_get_sleep)

        non_chat_final_transport_failures = max(0, state.transport_final_failures - state.chat_post_transport_ambiguous)
        if state.chat_post_transport_ambiguous > 0:
            problems.append("one or more /v1/chat/completions POSTs had transport failure and were not retried")
        if non_chat_final_transport_failures > 0:
            problems.append("one or more non-chat routes still had final transport failure after retries")
        if state.cli_failures:
            problems.extend(state.cli_failures)
        if state.semantic_failures:
            problems.extend(state.semantic_failures)
        if state.valid_chat_cases > 0 and state.valid_chat_2xx == 0:
            problems.append("no valid prompt-backed chat case returned a 2xx response")
        if state.transport_first_attempt_failures > 0:
            warnings.append("one or more HTTP requests had first-attempt transport failures that recovered on retry")
        if state.chat_valid_non_2xx > 0:
            warnings.append("one or more valid prompt-backed chat cases returned non-2xx")

        verdict = "failed" if problems else ("healthy_with_warnings" if warnings else "healthy")

    except KeyboardInterrupt:
        verdict = "interrupted"
        problems.append("runner interrupted")
        append_log(log_path, "\n# Runner interrupted\n\n")
    except Exception as exc:
        verdict = "runner_failed"
        problems.append(f"{type(exc).__name__}: {exc}")
        append_log(log_path, "\n# Runner exception\n\n" + fence("text", traceback.format_exc()))
    finally:
        progress("SUMMARY", f"verdict={verdict}")
        progress("SUMMARY", f"log file: {log_path}")
        for problem in problems[:20]:
            progress("PROBLEM", problem)
        for warning in warnings[:20]:
            progress("WARNING", warning)
        append_log(log_path, "\n# Final step 3 verdict\n\n")
        append_log(log_path, f"Finished UTC: `{utc_now()}`  \nVerdict: `{verdict}`\n\n")
        append_log(log_path, "## Summary data\n\n" + fence("json", json.dumps(state.as_dict(), indent=2, sort_keys=True)))
        append_log(log_path, "\n## Warnings\n\n" + ("".join(f"- {warning}\n" for warning in warnings) if warnings else "- none\n"))
        append_log(log_path, "\n## Problems\n\n" + ("".join(f"- {problem}\n" for problem in problems) if problems else "- none\n"))

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
