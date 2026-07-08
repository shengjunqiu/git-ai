#!/usr/bin/env python3
"""Benchmark /health and /ready endpoints."""

from __future__ import annotations

import argparse
import os

from _common import (
    api_headers,
    build_url,
    env_float,
    env_int,
    exit_if_failed,
    http_request,
    normalize_base_url,
    positive_int,
    print_summaries,
    run_concurrent,
    summarize,
    timed_request,
)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--base-url",
        default=os.environ.get("ENTERPRISE_BASE_URL", "http://127.0.0.1:8080"),
        help="Enterprise server base URL.",
    )
    parser.add_argument(
        "--requests",
        type=positive_int,
        default=env_int("BENCH_REQUESTS", 1000),
        help="Requests per endpoint.",
    )
    parser.add_argument(
        "--concurrency",
        type=positive_int,
        default=env_int("BENCH_CONCURRENCY", 20),
        help="Concurrent workers.",
    )
    parser.add_argument(
        "--timeout",
        type=float,
        default=env_float("BENCH_TIMEOUT_SECONDS", 10.0),
        help="Per-request timeout in seconds.",
    )
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    base_url = normalize_base_url(args.base_url)
    endpoints = [("health", "/health"), ("ready", "/ready")]
    headers = api_headers()
    total_requests = args.requests * len(endpoints)

    def work(index: int):
        label, path = endpoints[index % len(endpoints)]
        url = build_url(base_url, path)
        return timed_request(
            label,
            lambda: http_request("GET", url, headers=headers, timeout_s=args.timeout),
        )

    results, elapsed_s = run_concurrent(total_requests, args.concurrency, work)
    print_summaries(
        f"enterprise health/readiness benchmark elapsed_s={elapsed_s:.2f}",
        summarize(results, elapsed_s),
    )
    exit_if_failed(results)


if __name__ == "__main__":
    main()
