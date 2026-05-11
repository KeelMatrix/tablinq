#!/usr/bin/env python3
from __future__ import annotations

import argparse
import datetime as dt
import json
import os
import subprocess
import sys
import threading
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
        self.chat_post_transport_ambiguous = 0
        self.cli_failures: List[str] = []
        self.semantic_failures: List[str] = []
        self.warnings: List[str] = []
        self.upload_file_id: Optional[str] = None
        self.request_ids: Dict[str, str] = {}

    def record_http(self, label: str, result: Dict[str, Any], is_chat_post: bool = False) -> None:
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
            if is_chat_post:
                self.chat_post_transport_ambiguous += 1
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
            "chat_post_transport_ambiguous": self.chat_post_transport_ambiguous,
            "cli_failures": self.cli_failures,
            "semantic_failures": self.semantic_failures,
            "warnings": self.warnings,
            "upload_file_id": self.upload_file_id,
            "request_ids": self.request_ids,
        }


_log_lock = threading.Lock()


def now_local() -> str:
    return dt.datetime.now().strftime("%H:%M:%S")


def utc_now() -> str:
    return dt.datetime.now(dt.timezone.utc).isoformat(timespec="seconds")


def stamp() -> str:
    return dt.datetime.now().strftime("%Y%m%d-%H%M%S")


def run_id_stamp() -> str:
    return dt.datetime.now().strftime("step4-%Y%m%d-%H%M%S")


def progress(phase: str, message: str) -> None:
    print(f"[{now_local()}] {phase}: {message}", flush=True)


def append_log(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with _log_lock:
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
        body_text = body_bytes.decode("utf-8", errors="replace")
        parts.extend(["--data-raw", quote_for_curl(body_text)])
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


def prepare_request(
    method: str,
    root: str,
    path: str,
    api_key: str,
    body_obj: Optional[Any] = None,
    body_text: Optional[str] = None,
    body_bytes: Optional[bytes] = None,
    content_type: Optional[str] = "application/json",
    extra_headers: Optional[Dict[str, str]] = None,
    auth_mode: str = "valid",
) -> Tuple[str, Dict[str, str], Optional[bytes]]:
    if body_obj is not None and (body_text is not None or body_bytes is not None):
        raise ValueError("body_obj, body_text, and body_bytes are mutually exclusive")
    if body_text is not None and body_bytes is not None:
        raise ValueError("body_text and body_bytes are mutually exclusive")

    if body_obj is not None:
        body_text = json.dumps(body_obj, ensure_ascii=False, separators=(",", ":"))
    if body_text is not None:
        body_bytes = body_text.encode("utf-8")

    headers: Dict[str, str] = {"Accept": "application/json"}
    if auth_mode == "valid":
        headers["Authorization"] = f"Bearer {api_key}"
    elif auth_mode == "missing":
        pass
    elif auth_mode == "wrong":
        headers["Authorization"] = "Bearer definitely-wrong-step4-token"
    elif auth_mode == "malformed":
        headers["Authorization"] = "Bearer"
    else:
        raise ValueError(f"unknown auth_mode: {auth_mode}")

    if body_bytes is not None and content_type:
        headers["Content-Type"] = content_type
    if extra_headers:
        headers.update(extra_headers)

    return absolute_url(root, path), headers, body_bytes


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
    body_bytes: Optional[bytes] = None,
    content_type: Optional[str] = "application/json",
    no_transport_retry: bool = False,
    extra_headers: Optional[Dict[str, str]] = None,
    auth_mode: str = "valid",
    is_chat_post: bool = False,
) -> Dict[str, Any]:
    method = method.upper()
    url, headers, prepared_body = prepare_request(
        method,
        root,
        path,
        api_key,
        body_obj=body_obj,
        body_text=body_text,
        body_bytes=body_bytes,
        content_type=content_type,
        extra_headers=extra_headers,
        auth_mode=auth_mode,
    )

    curl_headers = dict(headers)
    if curl_headers.get("Authorization") == f"Bearer {api_key}":
        curl_headers["Authorization"] = "Bearer <REDACTED_API_KEY>"

    effective_retries = 0 if no_transport_retry else retries
    curl = build_curl(method, url, curl_headers, prepared_body)

    append_log(log_path, f"\n## {section} {index:03d}: `{method} {path}`\n\n")
    append_log(log_path, f"Note: {note}\n\n")
    append_log(log_path, f"Auth mode: `{auth_mode}`  \nRetries for transport errors: `{effective_retries}`  \nNo transport retry: `{str(no_transport_retry).lower()}`\n\n")
    append_log(log_path, "### cURL\n\n" + fence("bash", redact(curl, api_key)))

    attempts: List[Dict[str, Any]] = []
    for attempt in range(1, effective_retries + 2):
        progress(section, f"{index:03d} attempt {attempt}/{effective_retries + 1}: {method} {path}")
        result = raw_http(method, url, headers, prepared_body, timeout)
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
    state.record_http(f"{section} {index:03d} {method} {path}", final, is_chat_post=is_chat_post)
    return final


def start_background_request(
    log_path: Path,
    state: RunState,
    index: int,
    section: str,
    method: str,
    root: str,
    path: str,
    api_key: str,
    timeout: int,
    note: str,
    body_obj: Optional[Any],
    extra_headers: Optional[Dict[str, str]],
    is_chat_post: bool,
) -> Tuple[threading.Thread, Dict[str, Any]]:
    method = method.upper()
    url, headers, prepared_body = prepare_request(
        method,
        root,
        path,
        api_key,
        body_obj=body_obj,
        content_type="application/json",
        extra_headers=extra_headers,
        auth_mode="valid",
    )
    curl_headers = dict(headers)
    if curl_headers.get("Authorization") == f"Bearer {api_key}":
        curl_headers["Authorization"] = "Bearer <REDACTED_API_KEY>"
    curl = build_curl(method, url, curl_headers, prepared_body)

    container: Dict[str, Any] = {
        "result": None,
        "section": section,
        "index": index,
        "method": method,
        "path": path,
        "is_chat_post": is_chat_post,
    }

    append_log(log_path, f"\n## {section} {index:03d}: `{method} {path}` background\n\n")
    append_log(log_path, f"Note: {note}\n\n")
    append_log(log_path, "Retries for transport errors: `0`  \nNo transport retry: `true`  \nBackground request: `true`\n\n")
    append_log(log_path, "### cURL\n\n" + fence("bash", redact(curl, api_key)))

    def worker() -> None:
        progress(section, f"{index:03d} background start: {method} {path}")
        container["result"] = raw_http(method, url, headers, prepared_body, timeout)
        progress(section, f"{index:03d} background done: status={container['result'].get('status_code')}")

    thread = threading.Thread(target=worker, daemon=True)
    thread.start()
    return thread, container


def finalize_background_request(
    log_path: Path,
    state: RunState,
    thread: threading.Thread,
    container: Dict[str, Any],
    join_timeout: float,
    api_key: str,
) -> Optional[Dict[str, Any]]:
    section = str(container["section"])
    index = int(container["index"])
    method = str(container["method"])
    path = str(container["path"])
    thread.join(join_timeout)

    if thread.is_alive():
        append_log(log_path, f"\n### Background result for {section} {index:03d}\n\n")
        append_log(log_path, f"Background request did not finish within `{join_timeout}` seconds. The runner will continue, but this is a warning.\n")
        state.warnings.append(f"{section} {index:03d} {method} {path} did not finish within join timeout")
        return None

    result = container.get("result")
    if not isinstance(result, dict):
        state.semantic_failures.append(f"{section} {index:03d} {method} {path}: background result missing")
        return None

    result["attempts"] = [result]
    result["attempt_count"] = 1
    result["retried"] = False

    append_log(log_path, f"\n### Background result for {section} {index:03d}\n\n")
    append_log(log_path, f"Started UTC: `{result['started']}`  \nFinished UTC: `{result['finished']}`  \nStatus seen by smoke runner: `{result['status_code']}`  \nTransport error type: `{result['transport_error_type']}`\n\n")
    append_log(log_path, "#### Raw response headers\n\n" + fence("http", redact(result["headers"], api_key)))
    append_log(log_path, "\n#### Raw response body\n\n" + fence("text", redact(result["body"], api_key)))
    state.record_http(f"{section} {index:03d} {method} {path}", result, is_chat_post=bool(container.get("is_chat_post")))
    return result


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


def request_id_for(run_id: str, case_id: str) -> str:
    safe = "".join(ch if ch.isalnum() or ch in "-_" else "-" for ch in case_id.lower())
    return f"{run_id}-{safe}"


def create_upload_fixture(script_dir: Path, run_id: str, no_txt: bool) -> Tuple[Path, str, bytes]:
    suffix = "" if no_txt else ".txt"
    file_name = f"step4-upload-field-note-{run_id}{suffix}"
    path = script_dir / file_name
    content = (
        f"Step 4 upload fixture {run_id}\n"
        "Niche: neighborhood history walk.\n"
        "Observation: the bronze plaque near the old tram stop mentions a market hall that no longer exists.\n"
        "Question focus: ask for one gentle follow-up question, not a summary.\n"
    ).encode("utf-8")
    path.write_bytes(content)
    return path, file_name, content


def extract_file_id(result: Dict[str, Any]) -> Optional[str]:
    body = result.get("json")
    if isinstance(body, dict):
        data = body.get("data")
        if isinstance(data, dict):
            value = data.get("id")
            if isinstance(value, str) and value.strip():
                return value.strip()
    return None


def chat_body(model: str, prompt: str, run_id: str, case_id: str, **extra: Any) -> Dict[str, Any]:
    body: Dict[str, Any] = {
        "model": model,
        "stream": False,
        "tablinq_new_conversation": True,
        "metadata": {
            "step4_run_id": run_id,
            "step4_case_id": case_id,
        },
        "messages": [{"role": "user", "content": prompt}],
    }
    body.update(extra)
    return body


def followup_reads(log_path: Path, state: RunState, idx: int, root: str, api_key: str, timeout: int, retries: int, retry_delay: float, inter_get_sleep: float, ids: Dict[str, str], label: str) -> int:
    append_log(log_path, f"\n# Follow-up reads for {label}\n\n")
    request_id = ids.get("request_id")
    conversation_id = ids.get("conversation_id")
    session_id = ids.get("session_id")
    paths: List[Tuple[str, str]] = []
    if request_id:
        paths.append((f"/v1/requests/{id_segment(request_id)}", "Request detail."))
        paths.append((f"/v1/requests/{id_segment(request_id)}/evidence", "Request evidence."))
    paths.append(("/v1/usage", "Usage state."))
    if conversation_id:
        paths.append((f"/v1/conversations/{id_segment(conversation_id)}", "Conversation detail."))
        paths.append((f"/v1/conversations/{id_segment(conversation_id)}/messages", "Conversation messages."))
        paths.append((f"/v1/conversations/{id_segment(conversation_id)}/snapshot", "Conversation snapshot."))
    if session_id:
        paths.append((f"/v1/sessions/{id_segment(session_id)}/conversations", "Session conversations."))
    paths.extend([("/v1/requests", "Requests discovery."), ("/v1/jobs", "Jobs discovery."), ("/admin/status", "Admin/browser status.")])
    for path, note in paths:
        request_with_logging(log_path, state, idx, "FOLLOWUP", "GET", root, path, api_key, timeout, retries, retry_delay, f"{label}: {note}")
        idx += 1
        if inter_get_sleep > 0:
            time.sleep(inter_get_sleep)
    return idx


def add_header(log_path: Path, env_file: Path, root: str, api_key_name: str, run_id: str, args: argparse.Namespace) -> None:
    append_log(log_path, "# Tablinq Gateway Step 4 Robustness Smoke Log\n\n")
    append_log(log_path, f"Generated UTC: `{utc_now()}`  \nRun ID: `{run_id}`  \nRunner: `tablinq_step4_robustness.py`  \nPython: `{sys.version.split()[0]}`  \n")
    append_log(log_path, f"Working directory: `{Path.cwd()}`  \nenv.local path: `{env_file}`  \nGateway root base URL: `{root}`  \nAPI key source: `{api_key_name}`  \nAPI key value: `<REDACTED_API_KEY>`\n\n")
    append_log(log_path, "Scope: negative/malformed route probes, upload/file flow, cancellation/race behavior, concurrency/ordering, and cleanup verification. No deep research and no `tablinq-research` requests.\n\n")
    append_log(log_path, "Safety: prompt POSTs are not blindly retried after transport errors. GET/status follow-ups may be retried. `--notxt` uploads an extensionless file with `text/plain` content type.\n\n")
    append_log(log_path, f"NoTxt mode: `{str(args.notxt).lower()}`  \nBetween prompt sleep seconds: `{args.between_prompt_sleep}`  \nInter-GET sleep seconds: `{args.inter_get_sleep}`\n")


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Tablinq Step 4 robustness smoke.")
    parser.add_argument("--env-file", default=None)
    parser.add_argument("--base-url", default=None)
    parser.add_argument("--log-file", default=None)
    parser.add_argument("--gpt-bin", default=os.environ.get("TABLINQ_GPT_BIN", "gpt"))
    parser.add_argument("--notxt", action="store_true", help="Upload an extensionless text/plain file instead of a .txt file.")
    parser.add_argument("--chat-timeout", type=int, default=900)
    parser.add_argument("--get-timeout", type=int, default=30)
    parser.add_argument("--cli-timeout", type=int, default=120)
    parser.add_argument("--get-retries", type=int, default=2)
    parser.add_argument("--retry-delay", type=float, default=0.75)
    parser.add_argument("--between-prompt-sleep", type=float, default=10.0)
    parser.add_argument("--inter-get-sleep", type=float, default=0.6)
    parser.add_argument("--attach-convergence-timeout", type=int, default=25)
    parser.add_argument("--attach-convergence-interval", type=float, default=2.0)
    parser.add_argument("--cancel-delay", type=float, default=1.5)
    parser.add_argument("--concurrency-gap", type=float, default=1.0)
    parser.add_argument("--background-join-timeout", type=float, default=930.0)
    parser.add_argument("--skip-cli-attach", action="store_true")
    parser.add_argument("--skip-detach-restore", action="store_true")
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
    log_path = Path(args.log_file).expanduser().resolve() if args.log_file else script_dir / f"tablinq-step4-robustness-{stamp()}.md"
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

        append_log(log_path, "\n# A. Preflight and attach convergence\n\n")
        if not args.skip_cli_attach:
            run_cli(log_path, state, cli_idx, [args.gpt_bin, "status"], args.cli_timeout, api_key, "Status before CLI attach.", allow_nonzero=True); cli_idx += 1
            run_cli(log_path, state, cli_idx, [args.gpt_bin, "browser", "attach"], args.cli_timeout, api_key, "CLI attach before API convergence.", allow_nonzero=False); cli_idx += 1
            run_cli(log_path, state, cli_idx, [args.gpt_bin, "status"], args.cli_timeout, api_key, "Status after CLI attach.", allow_nonzero=True); cli_idx += 1
        else:
            append_log(log_path, "CLI attach skipped by `--skip-cli-attach`.\n")

        attach_result = request_with_logging(log_path, state, idx, "ATTACH", "POST", root, "/v1/browser/attach", api_key, args.get_timeout, args.get_retries, args.retry_delay, "API attach before Step 4 requests.", body_obj={})
        idx += 1
        if not api_attach_result_healthy(attach_result):
            warnings.append("API attach response did not itself report a fully healthy attached browser state")
            state.warnings.append(warnings[-1])

        idx, converged, failures = wait_convergence(log_path, state, idx, root, api_key, args.get_timeout, args.get_retries, args.retry_delay, args.attach_convergence_timeout, args.attach_convergence_interval, "Step 4 preflight")
        if not converged:
            state.semantic_failures.extend([f"preflight attach convergence failed: {failure}" for failure in failures])
            raise RuntimeError("preflight attach convergence failed; refusing to send prompt-backed requests")

        for path in ["/v1/models", "/v1/config", "/v1/status", "/v1/doctor"]:
            request_with_logging(log_path, state, idx, "PREFLIGHT", "GET", root, path, api_key, args.get_timeout, args.get_retries, args.retry_delay, f"Preflight route check for {path}.")
            idx += 1
            time.sleep(args.inter_get_sleep)

        append_log(log_path, "\n# B. Hard negative and malformed route probes\n\n")
        negative_cases = [
            ("GET", "/v1/status", None, None, "missing", "Missing auth against v1 status."),
            ("GET", "/v1/status", None, None, "wrong", "Wrong bearer token against v1 status."),
            ("GET", "/v1/status", None, None, "malformed", "Malformed bearer header against v1 status."),
            ("GET", "/v1/definitely-not-step4", None, None, "valid", "Unknown v1 route."),
            ("DELETE", "/v1/models", None, None, "valid", "Wrong method against models."),
            ("GET", "/v1/requests?limit=not-an-int&offset=-999", None, None, "valid", "Bad request list query."),
            ("GET", "/v1/files?limit=-1&offset=nope", None, None, "valid", "Bad file list query."),
            ("GET", "/v1/requests/step4-invalid-id", None, None, "valid", "Invalid request id detail."),
            ("GET", "/v1/requests/step4-invalid-id/evidence", None, None, "valid", "Invalid request evidence."),
            ("GET", "/v1/conversations/step4-invalid-id", None, None, "valid", "Invalid conversation detail."),
            ("GET", "/v1/conversations/step4-invalid-id/messages", None, None, "valid", "Invalid conversation messages."),
            ("GET", "/v1/conversations/step4-invalid-id/snapshot", None, None, "valid", "Invalid conversation snapshot."),
            ("GET", "/v1/files/step4-invalid-file", None, None, "valid", "Invalid file detail."),
            ("GET", "/v1/artifacts/step4-invalid-artifact", None, None, "valid", "Invalid artifact detail."),
        ]
        for method, path, body_obj, body_text, auth_mode, note in negative_cases:
            request_with_logging(log_path, state, idx, "NEGATIVE", method, root, path, api_key, args.get_timeout, args.get_retries, args.retry_delay, note, body_obj=body_obj, body_text=body_text, auth_mode=auth_mode)
            idx += 1
            time.sleep(args.inter_get_sleep)

        chat_invalid_cases = [
            ("chat-missing-auth", {"model": "tablinq-instant", "stream": False, "messages": [{"role": "user", "content": "A gardener forgot labels on seed trays. What is one safe next habit?"}]}, "missing", "Missing auth chat validation; should fail before browser submission."),
            ("chat-wrong-auth", {"model": "tablinq-instant", "stream": False, "messages": [{"role": "user", "content": "A potter sees tiny cracks before firing. Name one harmless check."}]}, "wrong", "Wrong auth chat validation; should fail before browser submission."),
            ("chat-missing-model", {"stream": False, "messages": [{"role": "user", "content": "What makes a bookmark useful during a library visit?"}]}, "valid", "Missing model validation."),
            ("chat-empty-messages", {"model": "tablinq-instant", "stream": False, "messages": []}, "valid", "Empty messages validation."),
            ("chat-invalid-role", {"model": "tablinq-instant", "stream": False, "messages": [{"role": "tool", "content": "This should not be submitted."}]}, "valid", "Invalid role validation."),
            ("chat-unknown-model", {"model": "tablinq-step4-imaginary", "stream": False, "messages": [{"role": "user", "content": "Name one thing to check before folding a paper map."}]}, "valid", "Unsupported model validation."),
            ("chat-bad-file-id", {"model": "tablinq-instant", "stream": False, "tablinq_new_conversation": True, "tablinq_file_ids": ["step4-not-a-real-file"], "messages": [{"role": "user", "content": "Use the uploaded clue, if any."}]}, "valid", "Invalid staged file id validation."),
        ]
        for case_id, body, auth_mode, note in chat_invalid_cases:
            body["metadata"] = {"step4_run_id": run_id, "step4_case_id": case_id}
            result = request_with_logging(
                log_path, state, idx, "CHAT_INVALID", "POST", root, "/v1/chat/completions", api_key,
                args.get_timeout, 0, args.retry_delay, note, body_obj=body,
                extra_headers={"X-Request-Id": request_id_for(run_id, case_id)},
                no_transport_retry=True, auth_mode=auth_mode, is_chat_post=True
            )
            idx += 1
            status = int(result.get("status_code", 0) or 0)
            if status == 0:
                state.semantic_failures.append(f"{case_id}: validation chat POST had transport failure")
            elif 200 <= status <= 299:
                state.semantic_failures.append(f"{case_id}: validation chat POST unexpectedly returned HTTP {status}")
            time.sleep(2.0)

        malformed_chat = request_with_logging(
            log_path, state, idx, "CHAT_INVALID", "POST", root, "/v1/chat/completions", api_key,
            args.get_timeout, 0, args.retry_delay, "Malformed JSON validation.",
            body_text="{\"model\":\"tablinq-instant\",\"messages\":[", content_type="application/json",
            extra_headers={"X-Request-Id": request_id_for(run_id, "chat-malformed-json")},
            no_transport_retry=True, is_chat_post=True
        )
        idx += 1
        if int(malformed_chat.get("status_code", 0) or 0) == 0:
            state.semantic_failures.append("chat-malformed-json: validation chat POST had transport failure")
        elif 200 <= int(malformed_chat.get("status_code", 0) or 0) <= 299:
            state.semantic_failures.append(f"chat-malformed-json unexpectedly returned HTTP {malformed_chat.get('status_code')}")
        time.sleep(2.0)

        wrong_type_chat = request_with_logging(
            log_path, state, idx, "CHAT_INVALID", "POST", root, "/v1/chat/completions", api_key,
            args.get_timeout, 0, args.retry_delay, "Wrong content type validation.",
            body_text="model=tablinq-instant&messages=not-json", content_type="text/plain",
            extra_headers={"X-Request-Id": request_id_for(run_id, "chat-wrong-content-type")},
            no_transport_retry=True, is_chat_post=True
        )
        idx += 1
        if int(wrong_type_chat.get("status_code", 0) or 0) == 0:
            state.semantic_failures.append("chat-wrong-content-type: validation chat POST had transport failure")
        elif 200 <= int(wrong_type_chat.get("status_code", 0) or 0) <= 299:
            state.semantic_failures.append(f"chat-wrong-content-type unexpectedly returned HTTP {wrong_type_chat.get('status_code')}")
        time.sleep(2.0)

        append_log(log_path, "\n# C. Upload and file flow\n\n")
        fixture_path, upload_name, upload_bytes = create_upload_fixture(script_dir, run_id, args.notxt)
        append_log(log_path, f"Upload fixture path: `{fixture_path}`  \n")
        append_log(log_path, f"Upload file name sent in x-file-name: `{upload_name}`  \n")
        append_log(log_path, f"Upload mode notxt: `{str(args.notxt).lower()}`\n\n")

        upload_result = request_with_logging(
            log_path, state, idx, "UPLOAD", "POST", root, "/v1/files", api_key,
            args.get_timeout, args.get_retries, args.retry_delay,
            "Stage a unique text/plain upload fixture. In --notxt mode, the file name is extensionless.",
            body_bytes=upload_bytes, content_type="text/plain", extra_headers={"X-File-Name": upload_name}
        )
        idx += 1
        if int(upload_result.get("status_code", 0) or 0) != 201:
            state.semantic_failures.append(f"upload fixture expected HTTP 201, got {upload_result.get('status_code')}")
        file_id = extract_file_id(upload_result)
        if file_id:
            state.upload_file_id = file_id
            append_log(log_path, f"\nExtracted staged file id: `{file_id}`\n")
        else:
            state.semantic_failures.append("upload fixture did not return data.id")

        request_with_logging(log_path, state, idx, "UPLOAD", "POST", root, "/v1/files", api_key, args.get_timeout, args.get_retries, args.retry_delay, "Upload without x-file-name should return structured 400.", body_bytes=b"missing file name", content_type="text/plain")
        idx += 1
        request_with_logging(log_path, state, idx, "UPLOAD", "GET", root, "/v1/files", api_key, args.get_timeout, args.get_retries, args.retry_delay, "List staged files after upload.")
        idx += 1
        if file_id:
            request_with_logging(log_path, state, idx, "UPLOAD", "GET", root, f"/v1/files/{id_segment(file_id)}", api_key, args.get_timeout, args.get_retries, args.retry_delay, "Fetch staged file detail.")
            idx += 1

            file_chat_case = "upload-file-chat"
            file_chat_body = chat_body(
                "tablinq-instant",
                "I uploaded a small note about a neighborhood history walk. Give one gentle follow-up question I could ask a local guide.",
                run_id,
                file_chat_case,
                tablinq_file_ids=[file_id],
            )
            file_chat = request_with_logging(
                log_path, state, idx, "UPLOAD_CHAT", "POST", root, "/v1/chat/completions", api_key,
                args.chat_timeout, 0, args.retry_delay,
                "Use the staged file id in a non-research chat request. This must not use deep research.",
                body_obj=file_chat_body,
                extra_headers={"X-Request-Id": request_id_for(run_id, file_chat_case)},
                no_transport_retry=True,
                is_chat_post=True,
            )
            idx += 1
            if int(file_chat.get("status_code", 0) or 0) == 0:
                state.semantic_failures.append("upload-file-chat had transport failure")
            elif int(file_chat.get("status_code", 0) or 0) >= 400:
                warnings.append(f"upload-file-chat returned HTTP {file_chat.get('status_code')}; inspect whether provider upload controls were unavailable")
                state.warnings.append(warnings[-1])
            ids = extract_ids(file_chat)
            idx = followup_reads(log_path, state, idx, root, api_key, args.get_timeout, args.get_retries, args.retry_delay, args.inter_get_sleep, ids, file_chat_case)
            time.sleep(args.between_prompt_sleep)

        append_log(log_path, "\n# D. Cancellation and race flow\n\n")
        cancel_case = "cancel-race-stargazing"
        cancel_request_id = request_id_for(run_id, cancel_case)
        state.request_ids[cancel_case] = cancel_request_id
        cancel_body = chat_body(
            "tablinq-thinking-extended",
            "Imagine a small observatory preparing a public stargazing night. Write twelve concise checks they should do before guests arrive.",
            run_id,
            cancel_case,
        )
        cancel_thread, cancel_container = start_background_request(
            log_path, state, idx, "CANCEL_POST", "POST", root, "/v1/chat/completions", api_key,
            args.chat_timeout,
            "Background prompt POST used for cancellation/race behavior. It is not retried.",
            cancel_body,
            {"X-Request-Id": cancel_request_id},
            True,
        )
        idx += 1
        progress("SLEEP", f"sleeping {args.cancel_delay}s before cancel")
        time.sleep(args.cancel_delay)
        cancel_result = request_with_logging(
            log_path, state, idx, "CANCEL", "POST", root, f"/v1/requests/{id_segment(cancel_request_id)}/cancel",
            api_key, args.get_timeout, args.get_retries, args.retry_delay,
            "Cancel the known request id while the background POST may still be running.",
            body_obj={}
        )
        idx += 1
        cancel_status = int(cancel_result.get("status_code", 0) or 0)
        if cancel_status not in (200, 404, 409):
            warnings.append(f"cancel request returned HTTP {cancel_status}; inspect semantics")
            state.warnings.append(warnings[-1])
        time.sleep(args.inter_get_sleep)
        for path in [f"/v1/requests/{id_segment(cancel_request_id)}", f"/v1/requests/{id_segment(cancel_request_id)}/evidence", "/v1/jobs", "/v1/usage", "/v1/conversations"]:
            request_with_logging(log_path, state, idx, "CANCEL_FOLLOWUP", "GET", root, path, api_key, args.get_timeout, args.get_retries, args.retry_delay, f"Cancellation follow-up read for {path}.")
            idx += 1
            time.sleep(args.inter_get_sleep)
        cancel_post_result = finalize_background_request(log_path, state, cancel_thread, cancel_container, args.background_join_timeout, api_key)
        if cancel_post_result is not None and int(cancel_post_result.get("status_code", 0) or 0) == 0:
            state.semantic_failures.append("cancel-race prompt POST had transport failure")

        append_log(log_path, "\n# E. Concurrency and ordering flow\n\n")
        concurrency_cases = [
            (
                "concurrent-mapmaking-a",
                "A cartographer is drawing a fantasy island map. Give one small detail that makes a harbor feel believable.",
                "tablinq-instant",
            ),
            (
                "concurrent-music-b",
                "A luthier is checking a quiet buzz in a handmade guitar. Name one harmless first inspection.",
                "tablinq-instant",
            ),
            (
                "concurrent-garden-c",
                "A rooftop gardener wants herbs to survive a windy balcony. Give one practical design idea.",
                "tablinq-instant",
            ),
        ]
        background_items: List[Tuple[threading.Thread, Dict[str, Any], str]] = []
        for case_id, prompt, model in concurrency_cases:
            req_id = request_id_for(run_id, case_id)
            state.request_ids[case_id] = req_id
            body = chat_body(model, prompt, run_id, case_id)
            thread, container = start_background_request(
                log_path, state, idx, "CONCURRENT_POST", "POST", root, "/v1/chat/completions", api_key,
                args.chat_timeout,
                f"Concurrent prompt case {case_id}. Not retried.",
                body,
                {"X-Request-Id": req_id},
                True,
            )
            background_items.append((thread, container, case_id))
            idx += 1
            progress("SLEEP", f"sleeping {args.concurrency_gap}s before next concurrent prompt")
            time.sleep(args.concurrency_gap)

        for thread, container, case_id in background_items:
            result = finalize_background_request(log_path, state, thread, container, args.background_join_timeout, api_key)
            if result is None:
                warnings.append(f"{case_id}: background POST did not finish within join timeout")
                state.warnings.append(warnings[-1])
            else:
                status = int(result.get("status_code", 0) or 0)
                if status == 0:
                    state.semantic_failures.append(f"{case_id}: concurrent prompt POST had transport failure")
                elif status >= 500:
                    warnings.append(f"{case_id}: concurrent prompt returned HTTP {status}; inspect queue/provider behavior")
                    state.warnings.append(warnings[-1])
                ids = extract_ids(result)
                idx = followup_reads(log_path, state, idx, root, api_key, args.get_timeout, args.get_retries, args.retry_delay, args.inter_get_sleep, ids, case_id)
            time.sleep(args.inter_get_sleep)

        request_with_logging(log_path, state, idx, "CONCURRENT_STATE", "GET", root, "/v1/requests", api_key, args.get_timeout, args.get_retries, args.retry_delay, "Requests after concurrency flow.")
        idx += 1
        request_with_logging(log_path, state, idx, "CONCURRENT_STATE", "GET", root, "/v1/jobs", api_key, args.get_timeout, args.get_retries, args.retry_delay, "Jobs after concurrency flow.")
        idx += 1
        request_with_logging(log_path, state, idx, "CONCURRENT_STATE", "GET", root, "/v1/usage", api_key, args.get_timeout, args.get_retries, args.retry_delay, "Usage after concurrency flow.")
        idx += 1

        append_log(log_path, "\n# F. Final cleanup and verification\n\n")
        for path in ["/health", "/admin/status", "/v1/status", "/v1/doctor", "/v1/usage", "/v1/requests", "/v1/jobs", "/v1/conversations", "/v1/files", "/v1/artifacts"]:
            request_with_logging(log_path, state, idx, "FINAL", "GET", root, path, api_key, args.get_timeout, args.get_retries, args.retry_delay, f"Final state inspection for {path}.")
            idx += 1
            time.sleep(args.inter_get_sleep)

        if not args.skip_detach_restore:
            request_with_logging(log_path, state, idx, "CLEANUP", "POST", root, "/v1/browser/detach", api_key, args.get_timeout, args.get_retries, args.retry_delay, "Optional non-destructive detach for cleanup verification.", body_obj={})
            idx += 1
            time.sleep(args.inter_get_sleep)
            request_with_logging(log_path, state, idx, "CLEANUP", "POST", root, "/v1/browser/attach", api_key, args.get_timeout, args.get_retries, args.retry_delay, "Restore attach after cleanup detach.", body_obj={})
            idx += 1
            idx, converged, failures = wait_convergence(log_path, state, idx, root, api_key, args.get_timeout, args.get_retries, args.retry_delay, args.attach_convergence_timeout, args.attach_convergence_interval, "final restore attach")
            if not converged:
                state.semantic_failures.extend([f"final restore attach convergence failed: {failure}" for failure in failures])
        else:
            append_log(log_path, "Detach/restore cleanup skipped by `--skip-detach-restore`.\n")

        if state.chat_post_transport_ambiguous > 0:
            problems.append("one or more /v1/chat/completions POSTs had transport failure and were not retried")
        non_chat_final_transport_failures = max(0, state.transport_final_failures - state.chat_post_transport_ambiguous)
        if non_chat_final_transport_failures > 0:
            problems.append("one or more non-chat routes still had final transport failure after retries")
        if state.cli_failures:
            problems.extend(state.cli_failures)
        if state.semantic_failures:
            problems.extend(state.semantic_failures)
        if state.transport_first_attempt_failures > 0:
            warnings.append("one or more HTTP requests had first-attempt transport failures that recovered on retry")
        if state.warnings:
            warnings.extend(state.warnings)

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
        for problem in problems[:30]:
            progress("PROBLEM", problem)
        for warning in warnings[:30]:
            progress("WARNING", warning)
        append_log(log_path, "\n# Final Step 4 verdict\n\n")
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
