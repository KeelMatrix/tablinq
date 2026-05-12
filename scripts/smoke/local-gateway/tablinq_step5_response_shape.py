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
        self.cli_failures: List[str] = []
        self.semantic_failures: List[str] = []
        self.warnings: List[str] = []
        self.skipped: List[str] = []
        self.dynamic_ids: Dict[str, List[str]] = {}

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
            "skipped": self.skipped,
            "dynamic_ids": self.dynamic_ids,
        }


_log_lock = threading.Lock()


def now_local() -> str:
    return dt.datetime.now().strftime("%H:%M:%S")


def utc_now() -> str:
    return dt.datetime.now(dt.timezone.utc).isoformat(timespec="seconds")


def stamp() -> str:
    return dt.datetime.now().strftime("%Y%m%d-%H%M%S")


def run_id_stamp() -> str:
    return dt.datetime.now().strftime("step5-%Y%m%d-%H%M%S")


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
        headers["Authorization"] = "Bearer definitely-wrong-step5-token"
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


def id_segment(value: str) -> str:
    return urllib.parse.quote(value, safe="")


def collect_evidence_request_ids(value: Any) -> List[str]:
    found: List[str] = []

    def remember(item: str) -> None:
        item = item.strip()
        if item and item not in found:
            found.append(item)

    def from_file_name(file_name: str) -> None:
        marker = "request-evidence-"
        suffix = ".json"
        name = file_name.rsplit("/", 1)[-1].strip()
        if name.startswith(marker) and name.endswith(suffix):
            remember(name[len(marker):-len(suffix)])

    def from_evidence_route(route: str) -> None:
        marker = "/v1/requests/"
        suffix = "/evidence"
        if marker not in route or suffix not in route:
            return
        tail = route.split(marker, 1)[1]
        request_id = tail.split(suffix, 1)[0]
        try:
            request_id = urllib.parse.unquote(request_id)
        except Exception:
            pass
        remember(request_id)

    def walk(node: Any) -> None:
        if isinstance(node, dict):
            for key, child in node.items():
                if isinstance(child, str):
                    if key == "file_name":
                        from_file_name(child)
                    elif key == "latest_request_evidence" or child.startswith("/v1/requests/"):
                        from_evidence_route(child)
                walk(child)
        elif isinstance(node, list):
            for child in node:
                walk(child)

    walk(value)
    return found


def body_object(state: RunState, label: str, result: Dict[str, Any]) -> Optional[Dict[str, Any]]:
    body = result.get("json")
    if not isinstance(body, dict):
        state.semantic_failures.append(f"{label}: expected JSON object response")
        return None
    return body


def expect_status(state: RunState, label: str, result: Dict[str, Any], expected: Iterable[int]) -> bool:
    status = int(result.get("status_code", 0) or 0)
    expected_set = set(expected)
    if status not in expected_set:
        state.semantic_failures.append(f"{label}: expected HTTP {sorted(expected_set)}, got {status}")
        return False
    return True


def expect_error_body(state: RunState, label: str, result: Dict[str, Any]) -> None:
    body = body_object(state, label, result)
    if body is None:
        return
    if not isinstance(body.get("error"), dict):
        state.semantic_failures.append(f"{label}: expected nested error object")
        return
    if body["error"].get("type") != "invalid_request_error":
        state.semantic_failures.append(f"{label}: expected invalid_request_error, got {body['error'].get('type')!r}")
    if not isinstance(body["error"].get("message"), str) or not body["error"].get("message"):
        state.semantic_failures.append(f"{label}: expected non-empty error.message")


def expect_list_shape(state: RunState, label: str, result: Dict[str, Any], object_value: Optional[str], expected_view: str, expected_compact: Optional[bool], expected_limit: Optional[int], expected_offset: int) -> None:
    if not expect_status(state, label, result, [200]):
        return
    body = body_object(state, label, result)
    if body is None:
        return
    if object_value is not None and body.get("object") != object_value:
        state.semantic_failures.append(f"{label}: expected object={object_value!r}, got {body.get('object')!r}")
    if not isinstance(body.get("request_id"), str) or not body.get("request_id"):
        state.semantic_failures.append(f"{label}: expected request_id")
    data = body.get("data")
    if not isinstance(data, list):
        state.semantic_failures.append(f"{label}: expected data array")
    pagination = body.get("pagination")
    if not isinstance(pagination, dict):
        state.semantic_failures.append(f"{label}: expected pagination object")
    else:
        if pagination.get("limit") != expected_limit:
            state.semantic_failures.append(f"{label}: expected pagination.limit={expected_limit!r}, got {pagination.get('limit')!r}")
        if pagination.get("offset") != expected_offset:
            state.semantic_failures.append(f"{label}: expected pagination.offset={expected_offset}, got {pagination.get('offset')!r}")
        returned = pagination.get("returned")
        if not isinstance(returned, int):
            state.semantic_failures.append(f"{label}: expected integer pagination.returned")
        elif isinstance(data, list) and returned != len(data):
            state.semantic_failures.append(f"{label}: expected returned={len(data)}, got {returned}")
        if "has_more" not in pagination or not isinstance(pagination.get("has_more"), bool):
            state.semantic_failures.append(f"{label}: expected boolean pagination.has_more")
        if "next_offset" not in pagination:
            state.semantic_failures.append(f"{label}: expected pagination.next_offset field")
        if "total" not in pagination and "total_items" not in pagination:
            state.semantic_failures.append(f"{label}: expected pagination total or total_items field")
    meta = body.get("meta")
    if not isinstance(meta, dict):
        state.semantic_failures.append(f"{label}: expected meta object")
    else:
        if meta.get("view") != expected_view:
            state.semantic_failures.append(f"{label}: expected meta.view={expected_view!r}, got {meta.get('view')!r}")
        if expected_compact is not None and meta.get("compact") is not expected_compact:
            state.semantic_failures.append(f"{label}: expected meta.compact={expected_compact!r}, got {meta.get('compact')!r}")
        if "truncated" not in meta or not isinstance(meta.get("truncated"), bool):
            state.semantic_failures.append(f"{label}: expected boolean meta.truncated")
        if not isinstance(meta.get("truncated_fields"), list):
            state.semantic_failures.append(f"{label}: expected meta.truncated_fields array")


def expect_summary_items_omit_heavy_fields(state: RunState, label: str, result: Dict[str, Any], heavy_fields: Iterable[str]) -> None:
    body = result.get("json")
    data = body.get("data") if isinstance(body, dict) else None
    if not isinstance(data, list) or not data:
        return
    first = data[0]
    if not isinstance(first, dict):
        state.semantic_failures.append(f"{label}: expected first data item to be an object")
        return
    for field in heavy_fields:
        if field in first:
            state.semantic_failures.append(f"{label}: summary item unexpectedly contains heavy field {field!r}")


def expect_detail_body(state: RunState, label: str, result: Dict[str, Any], expected_object: str, expected_view: str, expected_compact: bool) -> Optional[Dict[str, Any]]:
    if not expect_status(state, label, result, [200]):
        return None
    body = body_object(state, label, result)
    if body is None:
        return None
    if body.get("object") != expected_object:
        state.semantic_failures.append(f"{label}: expected object={expected_object!r}, got {body.get('object')!r}")
    if not isinstance(body.get("data"), dict):
        state.semantic_failures.append(f"{label}: expected data object")
    meta = body.get("meta")
    if not isinstance(meta, dict):
        state.semantic_failures.append(f"{label}: expected meta object")
    else:
        if meta.get("view") != expected_view:
            state.semantic_failures.append(f"{label}: expected meta.view={expected_view!r}, got {meta.get('view')!r}")
        if meta.get("compact") is not expected_compact:
            state.semantic_failures.append(f"{label}: expected meta.compact={expected_compact!r}, got {meta.get('compact')!r}")
        if not isinstance(meta.get("truncated_fields"), list):
            state.semantic_failures.append(f"{label}: expected meta.truncated_fields array")
    return body


def skip(state: RunState, log_path: Path, label: str, reason: str) -> None:
    message = f"{label}: {reason}"
    state.skipped.append(message)
    append_log(log_path, f"\n## Skipped `{label}`\n\nReason: {reason}\n")
    progress("SKIP", message)

def add_header(log_path: Path, env_file: Path, root: str, api_key_name: str, run_id: str, args: argparse.Namespace) -> None:
    append_log(log_path, "# Tablinq Gateway Step 5 Response-Shape Smoke Log\n\n")
    append_log(log_path, f"Generated UTC: `{utc_now()}`  \nRun ID: `{run_id}`  \nRunner: `tablinq_step5_response_shape.py`  \nPython: `{sys.version.split()[0]}`  \n")
    append_log(log_path, f"Working directory: `{Path.cwd()}`  \nenv.local path: `{env_file}`  \nGateway root base URL: `{root}`  \nAPI key source: `{api_key_name}`  \nAPI key value: `<REDACTED_API_KEY>`\n\n")
    append_log(log_path, "Scope: step 5 response-shape smoke. This run validates pagination, summary/detail/full view metadata, includes, compact snapshots, message order, usage paging, and invalid query contracts. It does not send prompts.\n\n")
    append_log(log_path, f"Inter-GET sleep seconds: `{args.inter_get_sleep}`  \nAttach convergence timeout seconds: `{args.attach_convergence_timeout}`\n")


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Tablinq step 5 response-shape smoke.")
    parser.add_argument("--env-file", default=None)
    parser.add_argument("--base-url", default=None)
    parser.add_argument("--log-file", default=None)
    parser.add_argument("--gpt-bin", default=os.environ.get("TABLINQ_GPT_BIN", "gpt"))
    parser.add_argument("--get-timeout", type=int, default=30)
    parser.add_argument("--cli-timeout", type=int, default=120)
    parser.add_argument("--get-retries", type=int, default=2)
    parser.add_argument("--retry-delay", type=float, default=0.75)
    parser.add_argument("--inter-get-sleep", type=float, default=0.4)
    parser.add_argument("--attach-convergence-timeout", type=int, default=25)
    parser.add_argument("--attach-convergence-interval", type=float, default=2.0)
    parser.add_argument("--skip-cli-attach", action="store_true")
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
    log_path = Path(args.log_file).expanduser().resolve() if args.log_file else script_dir / f"tablinq-step5-response-shape-{stamp()}.md"
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

        attach_result = request_with_logging(log_path, state, idx, "ATTACH", "POST", root, "/v1/browser/attach", api_key, args.get_timeout, args.get_retries, args.retry_delay, "API attach before Step 5 response-shape requests.", body_obj={})
        idx += 1
        if not api_attach_result_healthy(attach_result):
            warning = "API attach response did not itself report a fully healthy attached browser state"
            warnings.append(warning)
            state.warnings.append(warning)

        idx, converged, failures = wait_convergence(log_path, state, idx, root, api_key, args.get_timeout, args.get_retries, args.retry_delay, args.attach_convergence_timeout, args.attach_convergence_interval, "Step 5 preflight")
        if not converged:
            state.semantic_failures.extend([f"preflight attach convergence failed: {failure}" for failure in failures])
            raise RuntimeError("preflight attach convergence failed; refusing to run response-shape probes")

        append_log(log_path, "\n# B. Discovery with explicit summary pagination\n\n")
        discovery_cases = [
            ("requests", "/v1/requests?limit=2&offset=0&view=summary", "tablinq.request_list", 2, 0, False, ["usage", "warnings", "recovery_trace", "recovery_packet"]),
            ("jobs", "/v1/jobs?limit=2&offset=0&view=summary", "tablinq.job_list", 2, 0, False, []),
            ("conversations", "/v1/conversations?limit=2&offset=0&view=summary", "tablinq.conversation_list", 2, 0, False, ["last_prompt", "last_response", "tab_url"]),
            ("files", "/v1/files?limit=2&offset=0&view=summary", "list", 2, 0, False, ["linked_request_ids", "linked_conversation_ids"]),
            ("artifacts", "/v1/artifacts?limit=2&offset=0&view=summary", "list", 2, 0, False, ["linked_request_ids", "linked_job_ids"]),
        ]
        discovery_results: Dict[str, Dict[str, Any]] = {}
        for key, path, object_value, expected_limit, expected_offset, expected_compact, heavy_fields in discovery_cases:
            result = request_with_logging(log_path, state, idx, "DISCOVERY", "GET", root, path, api_key, args.get_timeout, args.get_retries, args.retry_delay, f"Explicit Step 5 summary pagination probe for {key}.")
            discovery_results[key] = result
            expect_list_shape(state, f"discovery {key}", result, object_value, "summary", expected_compact, expected_limit, expected_offset)
            expect_summary_items_omit_heavy_fields(state, f"discovery {key}", result, heavy_fields)
            idx += 1
            time.sleep(args.inter_get_sleep)

        append_log(log_path, "\n# C. Usage pagination and summary shapes\n\n")
        usage_summary = request_with_logging(log_path, state, idx, "USAGE", "GET", root, "/v1/usage?summary=true&limit=2&offset=0", api_key, args.get_timeout, args.get_retries, args.retry_delay, "Usage summary route with explicit durable pagination.")
        expect_list_shape(state, "usage summary", usage_summary, "tablinq.request_summary_list", "detail", True, 2, 0)
        expect_summary_items_omit_heavy_fields(state, "usage summary", usage_summary, ["recovery_trace", "recovery_packet", "warnings"])
        idx += 1
        time.sleep(args.inter_get_sleep)

        usage_offset_50 = request_with_logging(log_path, state, idx, "USAGE", "GET", root, "/v1/usage?summary=true&limit=2&offset=50", api_key, args.get_timeout, args.get_retries, args.retry_delay, "Usage summary route with offset past the historical 50-row default boundary.")
        expect_list_shape(state, "usage offset 50", usage_offset_50, "tablinq.request_summary_list", "detail", True, 2, 50)
        idx += 1
        time.sleep(args.inter_get_sleep)

        requests_body = discovery_results.get("requests", {}).get("json")
        conversations_body = discovery_results.get("conversations", {}).get("json")
        artifacts_body = discovery_results.get("artifacts", {}).get("json")
        state.dynamic_ids = {
            "requests": collect_string_ids(requests_body, ["request_id", "id"]),
            "conversations": collect_string_ids(conversations_body, ["conversation_id", "id"]),
            "evidence_requests": collect_evidence_request_ids(artifacts_body),
        }

        if not state.dynamic_ids["evidence_requests"]:
            evidence_artifacts = request_with_logging(log_path, state, idx, "DISCOVERY", "GET", root, "/v1/artifacts?limit=25&offset=0&view=summary", api_key, args.get_timeout, args.get_retries, args.retry_delay, "Extended artifact scan to find request evidence artifacts for Step 5 evidence pagination.")
            expect_list_shape(state, "discovery evidence artifacts", evidence_artifacts, "list", "summary", False, 25, 0)
            state.dynamic_ids["evidence_requests"] = collect_evidence_request_ids(evidence_artifacts.get("json"))
            idx += 1
            time.sleep(args.inter_get_sleep)

        append_log(log_path, "\n## Extracted dynamic IDs for Step 5 detail probes\n\n" + fence("json", json.dumps(state.dynamic_ids, indent=2, sort_keys=True)))

        append_log(log_path, "\n# D. Conversation detail, message order, and snapshot shape probes\n\n")
        conversation_id = state.dynamic_ids.get("conversations", [None])[0] if state.dynamic_ids.get("conversations") else None
        if conversation_id:
            conv_path = f"/v1/conversations/{id_segment(conversation_id)}?include=messages,snapshot"
            conv_detail = request_with_logging(log_path, state, idx, "CONVERSATION", "GET", root, conv_path, api_key, args.get_timeout, args.get_retries, args.retry_delay, "Conversation detail with embedded messages and snapshot includes.")
            conv_body = expect_detail_body(state, "conversation include messages snapshot", conv_detail, "tablinq.conversation", "detail", False)
            if conv_body:
                data = conv_body.get("data")
                if isinstance(data, dict):
                    if "messages" not in data or not isinstance(data.get("messages"), dict):
                        state.semantic_failures.append("conversation include messages snapshot: expected data.messages object")
                    else:
                        messages = data["messages"]
                        if not isinstance(messages.get("data"), list):
                            state.semantic_failures.append("conversation include messages snapshot: expected data.messages.data array")
                        if not isinstance(messages.get("pagination"), dict):
                            state.semantic_failures.append("conversation include messages snapshot: expected data.messages.pagination object")
                    if "snapshot" not in data or not isinstance(data.get("snapshot"), dict):
                        state.semantic_failures.append("conversation include messages snapshot: expected data.snapshot object")
                meta = conv_body.get("meta") if isinstance(conv_body, dict) else None
                include = meta.get("include") if isinstance(meta, dict) else None
                if not isinstance(include, list) or "messages" not in include or "snapshot" not in include:
                    state.semantic_failures.append("conversation include messages snapshot: expected meta.include to contain messages and snapshot")
            idx += 1
            time.sleep(args.inter_get_sleep)

            msg_asc = request_with_logging(log_path, state, idx, "MESSAGES", "GET", root, f"/v1/conversations/{id_segment(conversation_id)}/messages?limit=2&offset=0", api_key, args.get_timeout, args.get_retries, args.retry_delay, "Conversation messages first page in default ascending order.")
            expect_list_shape(state, "messages asc", msg_asc, "tablinq.conversation_message_list", "detail", False, 2, 0)
            msg_asc_body = msg_asc.get("json")
            if isinstance(msg_asc_body, dict) and isinstance(msg_asc_body.get("meta"), dict) and msg_asc_body["meta"].get("order") != "asc":
                state.semantic_failures.append(f"messages asc: expected meta.order='asc', got {msg_asc_body['meta'].get('order')!r}")
            idx += 1
            time.sleep(args.inter_get_sleep)

            msg_desc = request_with_logging(log_path, state, idx, "MESSAGES", "GET", root, f"/v1/conversations/{id_segment(conversation_id)}/messages?limit=2&offset=0&order=desc", api_key, args.get_timeout, args.get_retries, args.retry_delay, "Conversation messages first page in descending order.")
            expect_list_shape(state, "messages desc", msg_desc, "tablinq.conversation_message_list", "detail", False, 2, 0)
            msg_desc_body = msg_desc.get("json")
            if isinstance(msg_desc_body, dict) and isinstance(msg_desc_body.get("meta"), dict) and msg_desc_body["meta"].get("order") != "desc":
                state.semantic_failures.append(f"messages desc: expected meta.order='desc', got {msg_desc_body['meta'].get('order')!r}")
            idx += 1
            time.sleep(args.inter_get_sleep)

            snapshot_compact = request_with_logging(log_path, state, idx, "SNAPSHOT", "GET", root, f"/v1/conversations/{id_segment(conversation_id)}/snapshot?compact=true", api_key, args.get_timeout, args.get_retries, args.retry_delay, "Compact conversation snapshot should bound heavy recent message/debug/artifact state.")
            compact_body = expect_detail_body(state, "snapshot compact", snapshot_compact, "tablinq.conversation_snapshot", "detail", True)
            if compact_body and isinstance(compact_body.get("meta"), dict):
                meta = compact_body["meta"]
                if "max_items" not in meta or "returned_items" not in meta or "has_more" not in meta:
                    state.semantic_failures.append("snapshot compact: expected max_items, returned_items, and has_more meta fields")
                if isinstance(meta.get("max_items"), int) and isinstance(meta.get("returned_items"), int) and meta["returned_items"] > meta["max_items"]:
                    state.semantic_failures.append("snapshot compact: returned_items exceeds max_items")
            idx += 1
            time.sleep(args.inter_get_sleep)

            snapshot_full = request_with_logging(log_path, state, idx, "SNAPSHOT", "GET", root, f"/v1/conversations/{id_segment(conversation_id)}/snapshot?view=full", api_key, args.get_timeout, args.get_retries, args.retry_delay, "Full conversation snapshot should opt out of compact truncation.")
            expect_detail_body(state, "snapshot full", snapshot_full, "tablinq.conversation_snapshot", "full", False)
            idx += 1
            time.sleep(args.inter_get_sleep)
        else:
            skip(state, log_path, "conversation detail/message/snapshot probes", "no conversation ID found; Step 3 or Step 4 likely did not leave durable conversation state")

        append_log(log_path, "\n# E. Request detail, include evidence, and evidence pagination probes\n\n")
        request_id = state.dynamic_ids.get("requests", [None])[0] if state.dynamic_ids.get("requests") else None
        evidence_request_ids = state.dynamic_ids.get("evidence_requests") or []
        evidence_request_id = evidence_request_ids[0] if evidence_request_ids else None
        include_request_id = evidence_request_id or request_id

        if include_request_id:
            req_detail = request_with_logging(log_path, state, idx, "REQUEST", "GET", root, f"/v1/requests/{id_segment(include_request_id)}?view=summary&include=evidence", api_key, args.get_timeout, args.get_retries, args.retry_delay, "Request summary view with explicit evidence include.")
            req_body = expect_detail_body(state, "request summary include evidence", req_detail, "tablinq.request_summary", "summary", False)
            if req_body:
                data = req_body.get("data")
                if isinstance(data, dict) and "evidence" not in data:
                    state.semantic_failures.append("request summary include evidence: expected data.evidence field")
                if evidence_request_id and include_request_id == evidence_request_id and isinstance(data, dict) and data.get("evidence") is None:
                    state.semantic_failures.append("request summary include evidence: expected non-null data.evidence for evidence-bearing request")
                meta = req_body.get("meta")
                include = meta.get("include") if isinstance(meta, dict) else None
                if not isinstance(include, list) or "evidence" not in include:
                    state.semantic_failures.append("request summary include evidence: expected meta.include to contain evidence")
                if isinstance(data, dict) and "usage" in data:
                    state.semantic_failures.append("request summary include evidence: summary response unexpectedly contains usage field")
            idx += 1
            time.sleep(args.inter_get_sleep)
        else:
            skip(state, log_path, "request summary include evidence", "no request ID found; Step 3 or Step 4 likely did not leave durable request state")

        if evidence_request_id:
            evidence = request_with_logging(log_path, state, idx, "EVIDENCE", "GET", root, f"/v1/requests/{id_segment(evidence_request_id)}/evidence?limit=1", api_key, args.get_timeout, args.get_retries, args.retry_delay, "Request evidence pagination route using an evidence-bearing request discovered from evidence artifacts.")
            expect_status(state, "request evidence limit 1", evidence, [200])
            evidence_body = body_object(state, "request evidence limit 1", evidence)
            if evidence_body is not None:
                if evidence_body.get("object") != "tablinq.request_evidence":
                    state.semantic_failures.append(f"request evidence limit 1: expected object='tablinq.request_evidence', got {evidence_body.get('object')!r}")
                pagination = evidence_body.get("pagination")
                if not isinstance(pagination, dict):
                    state.semantic_failures.append("request evidence limit 1: expected pagination object")
                else:
                    if pagination.get("limit") != 1:
                        state.semantic_failures.append(f"request evidence limit 1: expected pagination.limit=1, got {pagination.get('limit')!r}")
                    if pagination.get("offset") != 0:
                        state.semantic_failures.append(f"request evidence limit 1: expected pagination.offset=0, got {pagination.get('offset')!r}")
                    returned = pagination.get("returned")
                    if not isinstance(returned, int):
                        state.semantic_failures.append("request evidence limit 1: expected integer pagination.returned")
                    elif returned < 1:
                        state.semantic_failures.append(f"request evidence limit 1: expected at least one evidence item, got returned={returned}")
                    if "has_more" not in pagination or not isinstance(pagination.get("has_more"), bool):
                        state.semantic_failures.append("request evidence limit 1: expected boolean pagination.has_more")
                    if "next_offset" not in pagination:
                        state.semantic_failures.append("request evidence limit 1: expected pagination.next_offset field")
                meta = evidence_body.get("meta")
                if not isinstance(meta, dict) or meta.get("view") != "detail" or meta.get("compact") is not False:
                    state.semantic_failures.append("request evidence limit 1: expected detail/compact=false meta")
                if evidence_body.get("data") is not None and not isinstance(evidence_body.get("data"), dict):
                    state.semantic_failures.append("request evidence limit 1: expected data object or null")
            idx += 1
            time.sleep(args.inter_get_sleep)
        else:
            skip(state, log_path, "request evidence pagination", "no evidence-bearing request was discovered from artifacts; Step 5 cannot truthfully exercise /v1/requests/{id}/evidence pagination")

        request_id = request_id or evidence_request_id

        append_log(log_path, "\n# F. Invalid response-shape query contracts\n\n")
        invalid_cases = [
            ("requests unsupported include", "/v1/requests?include=evidence"),
            ("requests unsupported summary", "/v1/requests?summary=true"),
            ("jobs unsupported order", "/v1/jobs?order=desc"),
            ("conversations unsupported summary", "/v1/conversations?summary=true"),
            ("files invalid compact bool", "/v1/files?compact=yes"),
            ("artifacts invalid view", "/v1/artifacts?view=tiny"),
            ("usage unsupported compact", "/v1/usage?compact=true"),
            ("usage invalid summary bool", "/v1/usage?summary=yes"),
        ]
        if conversation_id:
            invalid_cases.extend([
                ("conversation detail unsupported limit", f"/v1/conversations/{id_segment(conversation_id)}?limit=1"),
                ("conversation detail unsupported include", f"/v1/conversations/{id_segment(conversation_id)}?include=banana"),
                ("messages unsupported view", f"/v1/conversations/{id_segment(conversation_id)}/messages?view=summary"),
                ("messages invalid order", f"/v1/conversations/{id_segment(conversation_id)}/messages?order=newest"),
                ("snapshot unsupported order", f"/v1/conversations/{id_segment(conversation_id)}/snapshot?order=desc"),
                ("snapshot unsupported include", f"/v1/conversations/{id_segment(conversation_id)}/snapshot?include=snapshot"),
            ])
        else:
            invalid_cases.extend([
                ("missing conversation messages unsupported view", "/v1/conversations/step5-known-invalid-id/messages?view=summary"),
                ("missing conversation snapshot unsupported order", "/v1/conversations/step5-known-invalid-id/snapshot?order=desc"),
            ])
        if request_id:
            invalid_cases.extend([
                ("request detail unsupported limit", f"/v1/requests/{id_segment(request_id)}?limit=1"),
                ("request detail unsupported include", f"/v1/requests/{id_segment(request_id)}?include=banana"),
                ("request evidence unsupported view", f"/v1/requests/{id_segment(request_id)}/evidence?view=summary"),
            ])
        else:
            invalid_cases.extend([
                ("missing request detail unsupported limit", "/v1/requests/step5-known-invalid-id?limit=1"),
                ("missing request evidence unsupported view", "/v1/requests/step5-known-invalid-id/evidence?view=summary"),
            ])

        for label, path in invalid_cases:
            result = request_with_logging(log_path, state, idx, "INVALID", "GET", root, path, api_key, args.get_timeout, args.get_retries, args.retry_delay, f"{label}: should return a structured invalid_request_error.")
            expect_status(state, label, result, [400])
            expect_error_body(state, label, result)
            idx += 1
            time.sleep(args.inter_get_sleep)

        append_log(log_path, "\n# G. Final bounded state inspection\n\n")
        final_paths = [
            "/v1/requests?limit=3&view=summary",
            "/v1/jobs?limit=3&view=summary",
            "/v1/conversations?limit=3&view=summary",
            "/v1/files?limit=3&view=summary",
            "/v1/artifacts?limit=3&view=summary",
            "/v1/usage?summary=true&limit=3",
        ]
        for path in final_paths:
            request_with_logging(log_path, state, idx, "FINAL", "GET", root, path, api_key, args.get_timeout, args.get_retries, args.retry_delay, f"Final bounded response-shape inspection for {path}.")
            idx += 1
            time.sleep(args.inter_get_sleep)

        if state.transport_final_failures > 0:
            problems.append("one or more routes still had final transport failure after retries")
        if state.cli_failures:
            problems.extend(state.cli_failures)
        if state.semantic_failures:
            problems.extend(state.semantic_failures)
        if state.transport_first_attempt_failures > 0:
            warnings.append("one or more HTTP requests had first-attempt transport failures that recovered on retry")
        if state.warnings:
            warnings.extend([warning for warning in state.warnings if warning not in warnings])

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
        append_log(log_path, "\n# Final step 5 verdict\n\n")
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
