#!/usr/bin/env python3
"""Shared helpers for enterprise-server benchmark scripts."""

from __future__ import annotations

import argparse
import json
import math
import os
import statistics
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from concurrent.futures import FIRST_COMPLETED, ThreadPoolExecutor, wait
from dataclasses import dataclass
from typing import Any, Callable


MAX_METRICS_BATCH_SIZE = 500


@dataclass(frozen=True)
class RequestResult:
    label: str
    ok: bool
    status: int | None
    duration_ms: float
    error: str | None = None


@dataclass(frozen=True)
class Summary:
    label: str
    requests: int
    successes: int
    errors: int
    error_rate_pct: float
    rps: float
    avg_ms: float
    p50_ms: float
    p95_ms: float
    p99_ms: float
    min_ms: float
    max_ms: float


def env_int(name: str, default: int) -> int:
    raw = os.environ.get(name)
    if raw is None or raw == "":
        return default
    try:
        value = int(raw)
    except ValueError as exc:
        raise argparse.ArgumentTypeError(f"{name} must be an integer") from exc
    return value


def env_float(name: str, default: float) -> float:
    raw = os.environ.get(name)
    if raw is None or raw == "":
        return default
    try:
        return float(raw)
    except ValueError as exc:
        raise argparse.ArgumentTypeError(f"{name} must be a number") from exc


def positive_int(value: str) -> int:
    parsed = int(value)
    if parsed <= 0:
        raise argparse.ArgumentTypeError("value must be positive")
    return parsed


def normalize_base_url(value: str) -> str:
    return value.rstrip("/")


def build_url(base_url: str, path: str, params: dict[str, Any] | None = None) -> str:
    normalized_path = path if path.startswith("/") else f"/{path}"
    url = f"{normalize_base_url(base_url)}{normalized_path}"
    if not params:
        return url

    encoded = urllib.parse.urlencode(
        {key: value for key, value in params.items() if value is not None}
    )
    if encoded:
        return f"{url}?{encoded}"
    return url


def require_api_key(value: str | None) -> str:
    if value:
        return value
    print(
        "ERROR: ENTERPRISE_API_KEY is required for this benchmark.",
        file=sys.stderr,
    )
    sys.exit(2)


def require_api_keys(*values: str | None) -> list[str]:
    for value in values:
        if value:
            keys = [item.strip() for item in value.split(",") if item.strip()]
            if keys:
                return keys
    print(
        "ERROR: ENTERPRISE_API_KEY or ENTERPRISE_API_KEYS is required for this benchmark.",
        file=sys.stderr,
    )
    sys.exit(2)


def api_headers(api_key: str | None = None, distinct_id: str | None = None) -> dict[str, str]:
    headers = {
        "Accept": "application/json",
        "User-Agent": "git-ai-enterprise-bench/1.0",
    }
    if api_key:
        headers["X-API-Key"] = api_key
    if distinct_id:
        headers["X-Distinct-ID"] = distinct_id
    return headers


def http_request(
    method: str,
    url: str,
    *,
    headers: dict[str, str] | None = None,
    payload: Any | None = None,
    timeout_s: float = 30.0,
) -> tuple[int, str]:
    body = None
    request_headers = dict(headers or {})
    if payload is not None:
        body = json.dumps(payload, separators=(",", ":")).encode("utf-8")
        request_headers["Content-Type"] = "application/json"

    req = urllib.request.Request(
        url,
        data=body,
        headers=request_headers,
        method=method,
    )

    try:
        with urllib.request.urlopen(req, timeout=timeout_s) as response:
            raw_body = response.read().decode("utf-8", errors="replace")
            return response.status, raw_body
    except urllib.error.HTTPError as exc:
        raw_body = exc.read().decode("utf-8", errors="replace")
        return exc.code, raw_body


def timed_request(label: str, request_fn: Callable[[], tuple[int, str]]) -> RequestResult:
    start = time.perf_counter()
    try:
        status, body = request_fn()
        duration_ms = (time.perf_counter() - start) * 1000
        if 200 <= status < 300:
            return RequestResult(label, True, status, duration_ms)
        return RequestResult(label, False, status, duration_ms, body[:500])
    except Exception as exc:  # noqa: BLE001 - benchmark scripts must capture failures.
        duration_ms = (time.perf_counter() - start) * 1000
        return RequestResult(label, False, None, duration_ms, str(exc))


def timed_json_request(
    label: str,
    request_fn: Callable[[], tuple[int, str]],
    *,
    validate: Callable[[Any], str | None] | None = None,
) -> RequestResult:
    start = time.perf_counter()
    try:
        status, body = request_fn()
        duration_ms = (time.perf_counter() - start) * 1000
        if not 200 <= status < 300:
            return RequestResult(label, False, status, duration_ms, body[:500])

        try:
            parsed = json.loads(body) if body else {}
        except json.JSONDecodeError as exc:
            return RequestResult(label, False, status, duration_ms, f"invalid JSON: {exc}")

        if validate:
            validation_error = validate(parsed)
            if validation_error:
                return RequestResult(label, False, status, duration_ms, validation_error)

        return RequestResult(label, True, status, duration_ms)
    except Exception as exc:  # noqa: BLE001 - benchmark scripts must capture failures.
        duration_ms = (time.perf_counter() - start) * 1000
        return RequestResult(label, False, None, duration_ms, str(exc))


def run_concurrent(
    total_requests: int,
    concurrency: int,
    work: Callable[[int], RequestResult],
) -> tuple[list[RequestResult], float]:
    if total_requests <= 0:
        return [], 0.0
    if concurrency <= 0:
        raise ValueError("concurrency must be positive")

    results: list[RequestResult] = []
    started_at = time.perf_counter()
    next_index = 0

    with ThreadPoolExecutor(max_workers=concurrency) as executor:
        pending = set()
        while next_index < total_requests and len(pending) < concurrency:
            pending.add(executor.submit(work, next_index))
            next_index += 1

        while pending:
            done, pending = wait(pending, return_when=FIRST_COMPLETED)
            for future in done:
                results.append(future.result())
                if next_index < total_requests:
                    pending.add(executor.submit(work, next_index))
                    next_index += 1

    elapsed_s = time.perf_counter() - started_at
    return results, elapsed_s


def percentile(values: list[float], pct: float) -> float:
    if not values:
        return 0.0
    sorted_values = sorted(values)
    index = max(0, min(len(sorted_values) - 1, math.ceil((pct / 100.0) * len(sorted_values)) - 1))
    return sorted_values[index]


def summarize(results: list[RequestResult], elapsed_s: float) -> list[Summary]:
    labels = sorted({result.label for result in results})
    summaries: list[Summary] = []
    for label in labels:
        label_results = [result for result in results if result.label == label]
        durations = [result.duration_ms for result in label_results]
        requests = len(label_results)
        errors = sum(1 for result in label_results if not result.ok)
        successes = requests - errors
        summaries.append(
            Summary(
                label=label,
                requests=requests,
                successes=successes,
                errors=errors,
                error_rate_pct=(errors / requests * 100.0) if requests else 0.0,
                rps=(requests / elapsed_s) if elapsed_s > 0 else 0.0,
                avg_ms=statistics.mean(durations) if durations else 0.0,
                p50_ms=percentile(durations, 50),
                p95_ms=percentile(durations, 95),
                p99_ms=percentile(durations, 99),
                min_ms=min(durations) if durations else 0.0,
                max_ms=max(durations) if durations else 0.0,
            )
        )
    return summaries


def print_summaries(title: str, summaries: list[Summary]) -> None:
    print(title)
    print(
        "label,requests,successes,errors,error_rate_pct,rps,avg_ms,p50_ms,p95_ms,p99_ms,min_ms,max_ms"
    )
    for item in summaries:
        print(
            f"{item.label},"
            f"{item.requests},"
            f"{item.successes},"
            f"{item.errors},"
            f"{item.error_rate_pct:.2f},"
            f"{item.rps:.2f},"
            f"{item.avg_ms:.2f},"
            f"{item.p50_ms:.2f},"
            f"{item.p95_ms:.2f},"
            f"{item.p99_ms:.2f},"
            f"{item.min_ms:.2f},"
            f"{item.max_ms:.2f}"
        )


def print_sample_errors(results: list[RequestResult], limit: int = 5) -> None:
    errors = [result for result in results if not result.ok]
    if not errors:
        return
    print("\nSample errors:", file=sys.stderr)
    for result in errors[:limit]:
        print(
            f"- {result.label} status={result.status} duration_ms={result.duration_ms:.2f} "
            f"error={result.error}",
            file=sys.stderr,
        )


def exit_if_failed(results: list[RequestResult]) -> None:
    if any(not result.ok for result in results):
        print_sample_errors(results)
        sys.exit(1)


def parse_tool_models(raw: str | None) -> list[str]:
    if not raw:
        return ["codex::gpt-5", "cursor::claude-4", "copilot::gpt-4.1"]
    tool_models = [item.strip() for item in raw.split(",") if item.strip()]
    if not tool_models:
        raise argparse.ArgumentTypeError("tool model list cannot be empty")
    return tool_models


def make_committed_event(
    seed: int,
    *,
    now_s: int,
    days: int,
    repo_count: int,
    author_count: int,
    tool_models: list[str],
) -> dict[str, Any]:
    time_window_s = max(1, days * 24 * 60 * 60)
    timestamp = now_s - (seed % time_window_s)
    tool_model = tool_models[seed % len(tool_models)]
    ai_tool = 5 + (seed % 40)
    mixed_tool = seed % 9
    accepted_tool = max(0, ai_tool - (seed % 5))
    human = 3 + (seed % 30)
    deleted = seed % 8
    added = human + ai_tool + mixed_tool

    return {
        "t": timestamp,
        "e": 1,
        "v": {
            "0": human,
            "1": deleted,
            "2": added,
            "3": ["all", tool_model],
            "4": [mixed_tool, mixed_tool],
            "5": [ai_tool, ai_tool],
            "6": [accepted_tool, accepted_tool],
            "7": [ai_tool, ai_tool],
            "8": [0, 0],
            "9": [100 + (seed % 200), 50 + (seed % 100)],
            "10": max(0, timestamp - 60),
            "11": f"benchmark commit {seed}",
            "12": "generated by scripts/benchmarks/enterprise",
        },
        "a": {
            "0": "1.3.2-bench",
            "1": f"https://example.com/bench/repo-{seed % repo_count}.git",
            "2": f"bench-user-{seed % author_count}@example.com",
            "3": f"{seed:040x}"[-40:],
            "5": f"bench-branch-{seed % 8}",
            "20": tool_model.split("::", 1)[0],
            "21": tool_model.split("::", 1)[1] if "::" in tool_model else "",
            "22": f"bench-prompt-{seed % 1000}",
        },
    }


def make_metrics_batch(
    start_seed: int,
    count: int,
    *,
    now_s: int,
    days: int,
    repo_count: int,
    author_count: int,
    tool_models: list[str],
) -> dict[str, Any]:
    return {
        "v": 1,
        "events": [
            make_committed_event(
                start_seed + offset,
                now_s=now_s,
                days=days,
                repo_count=repo_count,
                author_count=author_count,
                tool_models=tool_models,
            )
            for offset in range(count)
        ],
    }
