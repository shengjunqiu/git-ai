#!/usr/bin/env python3
"""Benchmark dashboard aggregate endpoints."""

from __future__ import annotations

import argparse
import os
import time

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
    require_api_key,
    run_concurrent,
    summarize,
    timed_json_request,
)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--base-url",
        default=os.environ.get("ENTERPRISE_BASE_URL", "http://127.0.0.1:8080"),
        help="Enterprise server base URL.",
    )
    parser.add_argument(
        "--api-key",
        default=os.environ.get("ENTERPRISE_API_KEY"),
        help="Enterprise API key. Defaults to ENTERPRISE_API_KEY.",
    )
    parser.add_argument(
        "--requests",
        type=positive_int,
        default=env_int("BENCH_REQUESTS", 300),
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
        default=env_float("BENCH_TIMEOUT_SECONDS", 30.0),
        help="Per-request timeout in seconds.",
    )
    parser.add_argument(
        "--days",
        type=positive_int,
        default=env_int("BENCH_DAYS", 30),
        help="Dashboard time window in days.",
    )
    parser.add_argument(
        "--org",
        default=os.environ.get("BENCH_ORG"),
        help="Optional organization slug query parameter.",
    )
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    api_key = require_api_key(args.api_key)
    base_url = normalize_base_url(args.base_url)
    headers = api_headers(api_key)
    until = int(time.time())
    since = until - args.days * 24 * 60 * 60

    endpoints = [
        (
            "summary",
            "/api/v1/aggregate/summary",
            {"since": since, "until": until, "org": args.org},
        ),
        (
            "trends_ai_lines_day",
            "/api/v1/aggregate/trends",
            {
                "metric": "ai_lines",
                "granularity": "day",
                "since": since,
                "until": until,
                "org": args.org,
            },
        ),
        (
            "trends_ai_ratio_week",
            "/api/v1/aggregate/trends",
            {
                "metric": "ai_ratio",
                "granularity": "week",
                "since": since,
                "until": until,
                "org": args.org,
            },
        ),
        ("tools", "/api/v1/aggregate/tools", {}),
    ]
    total_requests = args.requests * len(endpoints)

    def work(index: int):
        label, path, params = endpoints[index % len(endpoints)]
        url = build_url(base_url, path, params)
        return timed_json_request(
            label,
            lambda: http_request("GET", url, headers=headers, timeout_s=args.timeout),
        )

    results, elapsed_s = run_concurrent(total_requests, args.concurrency, work)
    print_summaries(
        f"enterprise dashboard benchmark elapsed_s={elapsed_s:.2f}",
        summarize(results, elapsed_s),
    )
    exit_if_failed(results)


if __name__ == "__main__":
    main()
