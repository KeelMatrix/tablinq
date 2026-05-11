#!/usr/bin/env python3
"""
Tablinq gateway step 1.2 diagnostics.

Purpose:
- Diagnose mixed /health state after browser attach.
- Diagnose transient local transport aborts around /ready and browser profile-info.
- No chat/completions requests.
- No prompt submission.
- No deep research.
- Save raw cURL, raw response headers, raw response bodies, and short notes to one markdown file.
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


def find_key_paths(value: Any, key: str, prefix: str = "$") -> List[Tuple[str, Any]]:
    found: List[Tuple[str, Any]] = []

    if isinstance(value, dict):
        for k, v in value.items():
            p = f"{prefix}.{k}"
            if k == key:
                found.append((p, v))
            found.extend(find_key_paths(v, key, p))
    elif isinstance(value, list):
        for i, v in enumerate(value):
            found.extend(find_key_paths(v, key, f"{prefix}[{i}]"))

    return found


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
    error_type = None

    req = urllib.request.Request(url=url, data=body_bytes, headers=headers, method=method.upper())

    try:
        with urllib.request.urlopen(req, timeout=timeout_seconds) as response:
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


def request_with_retries(
    log_path: Path,
    index: int,
    label: str,
    method: str,
    root_base_url: str,
    path: str,
    api_key: str,
    timeout_seconds: int,
    retries: int,
    retry_delay_seconds: float,
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

    attempts: List[Dict[str, Any]] = []

    append_log(log_path, f"\n## {label} {index:03d}: `{method} {path}`\n\n")
    append_log(log_path, f"Note: {note}\n\n")
    append_log(log_path, "### cURL\n\n")
    append_log(log_path, fence("bash", curl_text))

    for attempt in range(1, retries + 2):
        progress(label, f"{index:03d} attempt {attempt}/{retries + 1}: {method} {path}")
        result = raw_http(method, url, headers, body_bytes, timeout_seconds)
        attempts.append(result)

        append_log(log_path, f"\n### Attempt {attempt}\n\n")
        append_log(log_path, f"Started UTC: `{result['started']}`  \n")
        append_log(log_path, f"Finished UTC: `{result['finished']}`  \n")
        append_log(log_path, f"Status seen by diagnostic runner: `{result['status_code']}`  \n")
        append_log(log_path, f"Transport error type: `{result['transport_error_type'] or ''}`\n\n")
        append_log(log_path, "#### Raw response headers\n\n")
        append_log(log_path, fence("http", redact(result["headers"], api_key)))
        append_log(log_path, "\n#### Raw response body\n\n")
        append_log(log_path, fence("text", redact(result["body"], api_key)))

        if int(result["status_code"]) != 0:
            break

        if attempt <= retries:
            progress(label, f"{index:03d} transport error; retrying after {retry_delay_seconds}s")
            time.sleep(retry_delay_seconds)

    final = attempts[-1]
    final["attempts"] = attempts
    final["attempt_count"] = len(attempts)
    final["retried"] = len(attempts) > 1
    return final


def classify_health(body_json: Any) -> Dict[str, Any]:
    if not isinstance(body_json, dict):
        return {
            "is_json_object": False,
            "top_level_status": None,
            "health_status_paths": [],
            "browser_connected_paths": [],
            "authenticated_paths": [],
            "extension_connected_paths": [],
            "mixed": False,
        }

    status_paths = find_key_paths(body_json, "status")
    browser_paths = find_key_paths(body_json, "browser_connected")
    auth_paths = find_key_paths(body_json, "authenticated")
    ext_paths = find_key_paths(body_json, "extension_connected")

    status_values = [v for _, v in status_paths if isinstance(v, str)]
    bool_values = []
    for _, v in browser_paths + auth_paths + ext_paths:
        if isinstance(v, bool):
            bool_values.append(v)

    mixed_status = "ok" in status_values and "degraded" in status_values
    mixed_bools = True in bool_values and False in bool_values

    return {
        "is_json_object": True,
        "top_level_status": body_json.get("status"),
        "health_status_paths": status_paths,
        "browser_connected_paths": browser_paths,
        "authenticated_paths": auth_paths,
        "extension_connected_paths": ext_paths,
        "mixed": bool(mixed_status or mixed_bools),
    }


def run_convergence_probe(
    log_path: Path,
    root_base_url: str,
    api_key: str,
    timeout_seconds: int,
    retries: int,
    retry_delay_seconds: float,
    rounds: int,
    round_delay_seconds: float,
) -> Dict[str, Any]:
    append_log(
        log_path,
        "\n# Diagnostic A: health convergence after attach\n\n"
        "This section polls `/health`, `/admin/status`, and `/v1/status` in sequence to see whether mixed health snapshots converge after attach.\n"
    )

    paths = ["/health", "/admin/status", "/v1/status"]
    observations: List[Dict[str, Any]] = []
    index = 1

    for round_no in range(1, rounds + 1):
        progress("CONVERGENCE", f"round {round_no}/{rounds}")
        append_log(log_path, f"\n## Convergence round {round_no}\n\n")

        round_obs: Dict[str, Any] = {"round": round_no, "paths": {}}

        for path in paths:
            result = request_with_retries(
                log_path=log_path,
                index=index,
                label="CONV",
                method="GET",
                root_base_url=root_base_url,
                path=path,
                api_key=api_key,
                timeout_seconds=timeout_seconds,
                retries=retries,
                retry_delay_seconds=retry_delay_seconds,
                note=f"Convergence probe round {round_no}: {path}.",
            )
            index += 1

            health = classify_health(result.get("json"))
            round_obs["paths"][path] = {
                "status_code": result["status_code"],
                "attempt_count": result["attempt_count"],
                "retried": result["retried"],
                "transport_error_type": result.get("transport_error_type"),
                "health_classification": health,
            }

            append_log(log_path, "\n#### Extracted health classification\n\n")
            append_log(log_path, fence("json", json.dumps(health, indent=2, default=str)))

        observations.append(round_obs)

        if round_no < rounds:
            time.sleep(round_delay_seconds)

    mixed_after_final = False
    final_round = observations[-1] if observations else {}
    for path_data in final_round.get("paths", {}).values():
        if path_data.get("health_classification", {}).get("mixed"):
            mixed_after_final = True

    return {
        "observations": observations,
        "mixed_after_final_round": mixed_after_final,
    }


def run_transport_stability_probe(
    log_path: Path,
    root_base_url: str,
    api_key: str,
    timeout_seconds: int,
    retries: int,
    retry_delay_seconds: float,
    loops: int,
    loop_delay_seconds: float,
) -> Dict[str, Any]:
    append_log(
        log_path,
        "\n# Diagnostic B: transport stability loop\n\n"
        "This section repeatedly calls the routes that previously showed local transport aborts, plus adjacent status routes. It records retries without hiding the first failed attempt.\n"
    )

    sequence = [
        "/ready",
        "/health",
        "/admin/status",
        "/v1/browser/profile-info",
        "/v1/browser/connection-info",
        "/v1/status",
    ]

    index = 1
    route_stats: Dict[str, Dict[str, int]] = {
        path: {
            "calls": 0,
            "transport_failures_first_attempt": 0,
            "transport_failures_final": 0,
            "retried_successes": 0,
            "http_successes_final": 0,
        }
        for path in sequence
    }

    for loop_no in range(1, loops + 1):
        progress("TRANSPORT", f"loop {loop_no}/{loops}")
        append_log(log_path, f"\n## Transport loop {loop_no}\n\n")

        for path in sequence:
            result = request_with_retries(
                log_path=log_path,
                index=index,
                label="STAB",
                method="GET",
                root_base_url=root_base_url,
                path=path,
                api_key=api_key,
                timeout_seconds=timeout_seconds,
                retries=retries,
                retry_delay_seconds=retry_delay_seconds,
                note=f"Transport stability loop {loop_no}: {path}.",
            )
            index += 1

            stats = route_stats[path]
            stats["calls"] += 1

            first_attempt = result["attempts"][0]
            if int(first_attempt["status_code"]) == 0:
                stats["transport_failures_first_attempt"] += 1

            if int(result["status_code"]) == 0:
                stats["transport_failures_final"] += 1
            else:
                if 200 <= int(result["status_code"]) <= 299:
                    stats["http_successes_final"] += 1
                if int(first_attempt["status_code"]) == 0:
                    stats["retried_successes"] += 1

            if loop_no < loops:
                time.sleep(loop_delay_seconds)

    return {
        "route_stats": route_stats,
    }


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Tablinq step 1.2 health/transport diagnostics.")
    parser.add_argument("--env-file", default=None, help="Path to .env.local. Defaults to .env.local beside this script.")
    parser.add_argument("--base-url", default=None, help="Gateway root base URL. Defaults to env or http://127.0.0.1:3847.")
    parser.add_argument("--log-file", default=None, help="Markdown log file path. Defaults to timestamped file beside this script.")
    parser.add_argument("--timeout", type=int, default=20, help="Per-request timeout seconds.")
    parser.add_argument("--retries", type=int, default=2, help="Transport retries per request. HTTP responses are not retried.")
    parser.add_argument("--retry-delay", type=float, default=0.75, help="Delay between transport retries.")
    parser.add_argument("--convergence-rounds", type=int, default=5, help="Number of convergence rounds.")
    parser.add_argument("--convergence-delay", type=float, default=1.5, help="Delay between convergence rounds.")
    parser.add_argument("--transport-loops", type=int, default=5, help="Number of transport stability loops.")
    parser.add_argument("--transport-delay", type=float, default=0.5, help="Delay between transport route calls.")
    parser.add_argument("--exit-zero-on-warnings", action="store_true", help="Return exit code 0 even when warnings are found.")
    return parser


def main(argv: Optional[Sequence[str]] = None) -> int:
    args = build_parser().parse_args(argv)

    script_dir = Path(__file__).resolve().parent
    env_file = Path(args.env_file).expanduser().resolve() if args.env_file else script_dir / ".env.local"
    env_values = parse_env_file(env_file)

    for k, v in env_values.items():
        os.environ.setdefault(k, v)

    api_key_name, api_key = first_present(API_KEY_ENV_NAMES, env_values)
    _base_name, base_url_from_env = first_present(BASE_URL_ENV_NAMES, env_values)
    root_base_url = normalize_root_base_url(args.base_url or base_url_from_env or DEFAULT_ROOT_BASE_URL)

    if args.log_file:
        log_path = Path(args.log_file).expanduser().resolve()
    else:
        log_path = script_dir / f"tablinq-step1-2-diagnostics-{filename_stamp()}.md"

    if log_path.exists():
        log_path.unlink()

    progress("SETUP", f"log file: {log_path}")
    progress("SETUP", f"env file: {env_file}")
    progress("SETUP", f"gateway URL: {root_base_url}")

    verdict = "runner_failed"
    problems: List[str] = []
    warnings: List[str] = []
    summary: Dict[str, Any] = {}

    try:
        if not api_key:
            raise RuntimeError("No API key found. Expected one of: " + ", ".join(API_KEY_ENV_NAMES))

        append_log(log_path, "# Tablinq Step 1.2 Health and Transport Diagnostics\n\n")
        append_log(log_path, f"Generated UTC: `{utc_now_iso()}`  \n")
        append_log(log_path, f"Runner: `{Path(__file__).name}`  \n")
        append_log(log_path, f"Python: `{sys.version.split()[0]}`  \n")
        append_log(log_path, f"Working directory: `{Path.cwd()}`  \n")
        append_log(log_path, f".env.local path: `{env_file}`  \n")
        append_log(log_path, f"Gateway root base URL: `{root_base_url}`  \n")
        append_log(log_path, f"API key source: `{api_key_name}`  \n")
        append_log(log_path, "API key value: `<REDACTED_API_KEY>`  \n\n")
        append_log(log_path, "Scope: no chat prompts, no research, health/transport diagnostics only.\n\n")

        convergence = run_convergence_probe(
            log_path=log_path,
            root_base_url=root_base_url,
            api_key=api_key,
            timeout_seconds=args.timeout,
            retries=args.retries,
            retry_delay_seconds=args.retry_delay,
            rounds=args.convergence_rounds,
            round_delay_seconds=args.convergence_delay,
        )

        stability = run_transport_stability_probe(
            log_path=log_path,
            root_base_url=root_base_url,
            api_key=api_key,
            timeout_seconds=args.timeout,
            retries=args.retries,
            retry_delay_seconds=args.retry_delay,
            loops=args.transport_loops,
            loop_delay_seconds=args.transport_delay,
        )

        summary = {
            "convergence": {
                "mixed_after_final_round": convergence["mixed_after_final_round"],
            },
            "transport": stability["route_stats"],
        }

        if convergence["mixed_after_final_round"]:
            warnings.append("health snapshots were still mixed after the final convergence round")

        for path, stats in stability["route_stats"].items():
            if stats["transport_failures_first_attempt"] > 0 and stats["transport_failures_final"] == 0:
                warnings.append(f"{path} had first-attempt transport failure(s) that recovered on retry")
            if stats["transport_failures_final"] > 0:
                problems.append(f"{path} still had final transport failure(s) after retries")

        if problems:
            verdict = "failed"
        elif warnings:
            verdict = "healthy_with_warnings"
        else:
            verdict = "healthy"

    except KeyboardInterrupt:
        verdict = "interrupted"
        problems.append("runner interrupted")
    except Exception as exc:
        verdict = "runner_failed"
        problems.append(f"{type(exc).__name__}: {exc}")
        append_log(log_path, "\n# Runner exception\n\n")
        append_log(log_path, fence("text", traceback.format_exc()))
    finally:
        progress("SUMMARY", f"verdict={verdict}")

        append_log(log_path, "\n# Final diagnostic verdict\n\n")
        append_log(log_path, f"Finished UTC: `{utc_now_iso()}`  \n")
        append_log(log_path, f"Verdict: `{verdict}`\n\n")

        append_log(log_path, "## Summary data\n\n")
        append_log(log_path, fence("json", json.dumps(summary, indent=2, sort_keys=True)))

        append_log(log_path, "\n## Warnings\n\n")
        if warnings:
            for warning in warnings:
                append_log(log_path, f"- {warning}\n")
                progress("WARNING", warning)
        else:
            append_log(log_path, "- none\n")

        append_log(log_path, "\n## Problems\n\n")
        if problems:
            for problem in problems:
                append_log(log_path, f"- {problem}\n")
                progress("PROBLEM", problem)
        else:
            append_log(log_path, "- none\n")

        progress("SUMMARY", f"log file: {log_path}")

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
