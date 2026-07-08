#!/usr/bin/env python3
"""Seed large metrics datasets through /worker/metrics/upload."""

from __future__ import annotations

import argparse
import math
import os
import time

from _common import (
    MAX_METRICS_BATCH_SIZE,
    api_headers,
    build_url,
    env_float,
    env_int,
    exit_if_failed,
    http_request,
    make_metrics_batch,
    normalize_base_url,
    parse_tool_models,
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
        "--events",
        type=positive_int,
        default=env_int("SEED_METRICS_EVENTS", 100000),
        help="Total committed metric events to upload.",
    )
    parser.add_argument(
        "--batch-size",
        type=positive_int,
        default=env_int("BENCH_BATCH_SIZE", MAX_METRICS_BATCH_SIZE),
        help=f"Events per upload request. Server limit is {MAX_METRICS_BATCH_SIZE}.",
    )
    parser.add_argument(
        "--concurrency",
        type=positive_int,
        default=env_int("BENCH_CONCURRENCY", 10),
        help="Concurrent upload workers.",
    )
    parser.add_argument(
        "--timeout",
        type=float,
        default=env_float("BENCH_TIMEOUT_SECONDS", 60.0),
        help="Per-request timeout in seconds.",
    )
    parser.add_argument(
        "--days",
        type=positive_int,
        default=env_int("BENCH_DAYS", 30),
        help="Spread generated event timestamps across this many days.",
    )
    parser.add_argument(
        "--repo-count",
        type=positive_int,
        default=env_int("BENCH_REPO_COUNT", 1000),
        help="Number of synthetic repo URLs.",
    )
    parser.add_argument(
        "--author-count",
        type=positive_int,
        default=env_int("BENCH_AUTHOR_COUNT", 1000),
        help="Number of synthetic commit authors.",
    )
    parser.add_argument(
        "--tool-models",
        default=os.environ.get("BENCH_TOOL_MODELS"),
        help="Comma-separated tool::model values.",
    )
    parser.add_argument(
        "--distinct-id",
        default=os.environ.get("BENCH_DISTINCT_ID", "enterprise-bench-seed"),
        help="X-Distinct-ID request header.",
    )
    parser.add_argument(
        "--start-seed",
        type=int,
        default=env_int("BENCH_START_SEED", 1),
        help="Seed offset for generated commit SHAs.",
    )
    return parser.parse_args()


def validate_metrics_response(parsed) -> str | None:
    errors = parsed.get("errors", []) if isinstance(parsed, dict) else []
    if errors:
        return f"metrics upload returned {len(errors)} event errors"
    return None


def main() -> None:
    args = parse_args()
    if args.batch_size > MAX_METRICS_BATCH_SIZE:
        raise SystemExit(f"--batch-size must be <= {MAX_METRICS_BATCH_SIZE}")

    api_key = require_api_key(args.api_key)
    base_url = normalize_base_url(args.base_url)
    url = build_url(base_url, "/worker/metrics/upload")
    headers = api_headers(api_key, args.distinct_id)
    tool_models = parse_tool_models(args.tool_models)
    now_s = int(time.time())
    batches = math.ceil(args.events / args.batch_size)

    def work(index: int):
        start_seed = args.start_seed + index * args.batch_size
        remaining = args.events - index * args.batch_size
        count = min(args.batch_size, remaining)
        payload = make_metrics_batch(
            start_seed,
            count,
            now_s=now_s,
            days=args.days,
            repo_count=args.repo_count,
            author_count=args.author_count,
            tool_models=tool_models,
        )
        return timed_json_request(
            "seed_metrics",
            lambda: http_request(
                "POST",
                url,
                headers=headers,
                payload=payload,
                timeout_s=args.timeout,
            ),
            validate=validate_metrics_response,
        )

    results, elapsed_s = run_concurrent(batches, args.concurrency, work)
    print(
        f"seeded_events_target={args.events} batches={batches} "
        f"batch_size={args.batch_size}"
    )
    print_summaries(
        f"enterprise metrics seed elapsed_s={elapsed_s:.2f}",
        summarize(results, elapsed_s),
    )
    exit_if_failed(results)


if __name__ == "__main__":
    main()
