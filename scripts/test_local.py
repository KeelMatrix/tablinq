#!/usr/bin/env python3
"""Run all local Tablinq tests with bounded parallelism and useful logs."""

from __future__ import annotations

import argparse
import os
import shutil
import subprocess
import sys
import tempfile
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable


@dataclass(frozen=True)
class Job:
    name: str
    command: tuple[str, ...]
    timeout_seconds: int | None = None


@dataclass(frozen=True)
class JobResult:
    job: Job
    returncode: int
    elapsed_seconds: float
    log_path: Path


RUST_CORE_JOBS = [
    Job("rust:gpt", ("cargo", "test", "--quiet", "-p", "gpt", "--locked"), 360),
    Job("rust:gptd", ("cargo", "test", "--quiet", "-p", "gptd", "--locked"), 300),
    Job(
        "rust:shared-protocol",
        ("cargo", "test", "--quiet", "-p", "shared-protocol", "--locked"),
        120,
    ),
    Job(
        "rust:tablinq-host",
        ("cargo", "test", "--quiet", "-p", "tablinq-host", "--locked"),
        120,
    ),
]

GATEWAY_FAST_FILTERS = [
    "auth::",
    "errors::",
    "openai::",
    "routes::",
    "tests::conversation_inspection",
    "tests::gateway_process",
    "tests::request_parsing",
]

GATEWAY_BROWSER_FILTERS = [
    "tests::capabilities_provider",
    "tests::chat_requests",
    "tests::failure_recovery_status",
    "tests::scheduler_process_control",
    "tests::uploads_artifacts",
]

TYPESCRIPT_JOBS = [
    Job("typescript:vitest", ("pnpm", "exec", "vitest", "run", "--reporter=dot"), 420),
]


def repo_root() -> Path:
    return Path(__file__).resolve().parents[1]


def add_cargo_bin_to_path(env: dict[str, str]) -> None:
    cargo_bin = Path.home() / ".cargo" / "bin"
    if cargo_bin.exists():
        env["PATH"] = f"{cargo_bin}{os.pathsep}{env.get('PATH', '')}"


def command_display(command: Iterable[str]) -> str:
    return " ".join(command)


def resolve_command(command: tuple[str, ...]) -> tuple[str, ...]:
    if os.name == "nt":
        for suffix in (".exe", ".cmd", ".bat"):
            executable = shutil.which(f"{command[0]}{suffix}")
            if executable is not None:
                return (executable, *command[1:])
    executable = shutil.which(command[0])
    if executable is None:
        raise FileNotFoundError(command[0])
    return (executable, *command[1:])


def ensure_tools() -> None:
    missing = [tool for tool in ("cargo", "pnpm") if shutil.which(tool) is None]
    if missing:
        raise SystemExit(f"Missing required command(s): {', '.join(missing)}")


def ensure_node_modules(root: Path, env: dict[str, str]) -> None:
    if (root / "node_modules").exists():
        return
    print("> pnpm install --frozen-lockfile", flush=True)
    subprocess.run(
        resolve_command(("pnpm", "install", "--frozen-lockfile")),
        cwd=root,
        env=env,
        check=True,
    )


def tail(path: Path, lines: int = 80) -> str:
    try:
        content = path.read_text(encoding="utf-8", errors="replace").splitlines()
    except FileNotFoundError:
        return ""
    return "\n".join(content[-lines:])


def run_job(job: Job, root: Path, log_dir: Path, env: dict[str, str]) -> JobResult:
    log_path = log_dir / f"{job.name.replace(':', '_')}.log"
    started = time.monotonic()
    with log_path.open("w", encoding="utf-8", errors="replace") as log:
        log.write(f"> {command_display(job.command)}\n\n")
        log.flush()
        try:
            process = subprocess.Popen(
                resolve_command(job.command),
                cwd=root,
                env=env,
                stdout=log,
                stderr=subprocess.STDOUT,
                text=True,
            )
        except Exception:
            log.close()
            raise
        try:
            returncode = process.wait(timeout=job.timeout_seconds)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
            log.write(
                f"\nCommand timed out after {job.timeout_seconds} seconds: "
                f"{command_display(job.command)}\n"
            )
            returncode = 124
    return JobResult(job, returncode, time.monotonic() - started, log_path)


def run_phase(
    name: str,
    jobs: list[Job],
    root: Path,
    log_dir: Path,
    env: dict[str, str],
    max_parallel: int,
) -> list[JobResult]:
    print(f"==> {name}", flush=True)
    pending = list(jobs)
    running: list[tuple[Job, subprocess.Popen[None], Path, float]] = []
    results: list[JobResult] = []

    def start(job: Job) -> None:
        log_path = log_dir / f"{job.name.replace(':', '_')}.log"
        log = log_path.open("w", encoding="utf-8", errors="replace")
        log.write(f"> {command_display(job.command)}\n\n")
        log.flush()
        try:
            process = subprocess.Popen(
                resolve_command(job.command),
                cwd=root,
                env=env,
                stdout=log,
                stderr=subprocess.STDOUT,
                text=True,
            )
        except Exception:
            log.close()
            raise
        process._tablinq_log = log  # type: ignore[attr-defined]
        print(f"  start {job.name}: {command_display(job.command)}", flush=True)
        running.append((job, process, log_path, time.monotonic()))

    try:
        while pending or running:
            while pending and len(running) < max_parallel:
                start(pending.pop(0))

            time.sleep(0.2)
            still_running: list[tuple[Job, subprocess.Popen[None], Path, float]] = []
            for job, process, log_path, started in running:
                timed_out = (
                    job.timeout_seconds is not None
                    and time.monotonic() - started > job.timeout_seconds
                )
                if timed_out:
                    process.kill()
                returncode = process.poll()
                if returncode is None:
                    still_running.append((job, process, log_path, started))
                    continue

                log = getattr(process, "_tablinq_log")
                if timed_out:
                    log.write(
                        f"\nCommand timed out after {job.timeout_seconds} seconds: "
                        f"{command_display(job.command)}\n"
                    )
                    returncode = 124
                log.close()
                elapsed = time.monotonic() - started
                print(
                    f"  {'ok' if returncode == 0 else 'fail'} {job.name} ({elapsed:.1f}s)",
                    flush=True,
                )
                results.append(JobResult(job, returncode, elapsed, log_path))
            running = still_running

            failures = [result for result in results if result.returncode != 0]
            if failures:
                for _, process, _, _ in running:
                    process.kill()
                for _, process, _, _ in running:
                    process.wait()
                    getattr(process, "_tablinq_log").close()
                break
    except Exception:
        for _, process, _, _ in running:
            if process.poll() is None:
                process.kill()
            process.wait()
            getattr(process, "_tablinq_log").close()
        raise

    return results


def gateway_job(filter_name: str, timeout_seconds: int) -> Job:
    return Job(
        f"gateway:{filter_name.replace('tests::', '').replace('::', '-')}",
        ("cargo", "test", "--quiet", "-p", "tablinq-gateway", "--locked", filter_name),
        timeout_seconds,
    )


def browser_gateway_jobs() -> list[Job]:
    return [
        gateway_job("tests::capabilities_provider", 180),
        gateway_job("tests::chat_requests", 240),
        gateway_job("tests::failure_recovery_status", 240),
        gateway_job("tests::scheduler_process_control", 240),
        gateway_job("tests::uploads_artifacts", 240),
    ]


def build_phases(profile: str) -> list[tuple[str, list[Job], int]]:
    gateway_fast = [gateway_job(filter_name, 120) for filter_name in GATEWAY_FAST_FILTERS]
    gateway_browser = browser_gateway_jobs()

    if profile == "aggressive":
        return [
            ("parallel core", RUST_CORE_JOBS + TYPESCRIPT_JOBS + gateway_fast, 6),
            ("browser-backed gateway", gateway_browser, 1),
        ]

    return [
        ("parallel unit/core", RUST_CORE_JOBS + gateway_fast, 4),
        (
            "typescript + browser-backed gateway",
            TYPESCRIPT_JOBS
            + [
                Job(
                    "gateway:browser-backed",
                    (
                        sys.executable,
                        str(Path(__file__).resolve()),
                        "--only-browser-gateway",
                    ),
                    900,
                )
            ],
            2,
        ),
    ]


def main() -> int:
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    if hasattr(sys.stderr, "reconfigure"):
        sys.stderr.reconfigure(encoding="utf-8", errors="replace")

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--profile",
        choices=("aggressive", "balanced"),
        default="balanced",
        help="parallelization profile to use",
    )
    parser.add_argument(
        "--keep-logs",
        action="store_true",
        help="keep per-job logs after success",
    )
    parser.add_argument(
        "--only-browser-gateway",
        action="store_true",
        help=argparse.SUPPRESS,
    )
    args = parser.parse_args()

    root = repo_root()
    env = os.environ.copy()
    add_cargo_bin_to_path(env)
    env.setdefault("CARGO_TERM_COLOR", "always")
    env.setdefault("RUST_BACKTRACE", "1")

    ensure_tools()
    ensure_node_modules(root, env)

    started = time.monotonic()
    with tempfile.TemporaryDirectory(prefix="tablinq-tests-") as temp:
        log_dir = Path(temp)
        print(f"logs: {log_dir}", flush=True)
        all_results: list[JobResult] = []
        phases = (
            [("browser-backed gateway", browser_gateway_jobs(), 1)]
            if args.only_browser_gateway
            else build_phases(args.profile)
        )

        for phase_name, jobs, max_parallel in phases:
            results = run_phase(phase_name, jobs, root, log_dir, env, max_parallel)
            all_results.extend(results)
            failures = [result for result in results if result.returncode != 0]
            if failures:
                print("\nFailures:", flush=True)
                for failure in failures:
                    print(
                        f"\n[{failure.job.name}] exit {failure.returncode}; "
                        f"log: {failure.log_path}",
                        flush=True,
                    )
                    print(tail(failure.log_path), flush=True)
                return 1

        elapsed = time.monotonic() - started
        print("\nSummary:", flush=True)
        for result in all_results:
            print(f"  {result.job.name}: {result.elapsed_seconds:.1f}s", flush=True)
        print(f"All local tests passed in {elapsed / 60:.0f}:{elapsed % 60:02.0f}.", flush=True)

        if args.keep_logs:
            keep_dir = root / "logs" / "local-test"
            keep_dir.mkdir(parents=True, exist_ok=True)
            for result in all_results:
                shutil.copy2(result.log_path, keep_dir / result.log_path.name)
            print(f"logs kept in {keep_dir}", flush=True)

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
