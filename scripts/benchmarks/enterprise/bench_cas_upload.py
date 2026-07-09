#!/usr/bin/env python3
"""Benchmark /worker/cas/upload with generated prompt records."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import time
from typing import Any

from _common import (
    api_headers,
    build_url,
    env_float,
    env_int,
    exit_if_failed,
    http_request,
    normalize_base_url,
    parse_tool_models,
    positive_int,
    print_sample_errors,
    print_summaries,
    require_api_keys,
    run_concurrent,
    summarize,
    timed_json_request,
)


MAX_CAS_UPLOAD_OBJECTS = 100


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
        "--api-keys",
        default=os.environ.get("ENTERPRISE_API_KEYS"),
        help="Comma-separated API keys. Defaults to ENTERPRISE_API_KEYS.",
    )
    parser.add_argument(
        "--requests",
        type=positive_int,
        default=env_int("BENCH_REQUESTS", 200),
        help="Number of CAS upload requests.",
    )
    parser.add_argument(
        "--objects-per-request",
        type=positive_int,
        default=env_int("BENCH_CAS_OBJECTS_PER_REQUEST", 10),
        help=f"CAS objects per request. Server limit is {MAX_CAS_UPLOAD_OBJECTS}.",
    )
    parser.add_argument(
        "--content-bytes",
        type=positive_int,
        default=env_int("BENCH_CAS_CONTENT_BYTES", 2048),
        help="Approximate prompt message content bytes per object.",
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
        "--tool-models",
        default=os.environ.get("BENCH_TOOL_MODELS"),
        help="Comma-separated tool::model values.",
    )
    parser.add_argument(
        "--distinct-id",
        default=os.environ.get("BENCH_DISTINCT_ID", "enterprise-bench-cas"),
        help="X-Distinct-ID request header.",
    )
    parser.add_argument(
        "--start-seed",
        type=int,
        default=env_int("BENCH_START_SEED", int(time.time())),
        help="Seed offset for generated CAS records.",
    )
    parser.add_argument(
        "--allow-errors",
        action="store_true",
        help="Print error statistics but exit 0 even when requests fail.",
    )
    parser.add_argument(
        "--error-samples",
        type=int,
        default=env_int("BENCH_ERROR_SAMPLES", 5),
        help="Number of sample errors to print when errors occur. Use 0 to disable.",
    )
    return parser.parse_args()


def canonical_json_string(value: Any) -> str:
    return json.dumps(
        value,
        sort_keys=True,
        separators=(",", ":"),
        ensure_ascii=False,
        allow_nan=False,
    )


def sha256_hex(value: str) -> str:
    return hashlib.sha256(value.encode("utf-8")).hexdigest()


def make_prompt_text(seed: int, target_bytes: int) -> str:
    prefix = f"benchmark prompt record {seed}. "
    chunk = "This generated prompt text is intentionally low entropy and contains no secrets. "
    if target_bytes <= len(prefix):
        return prefix[:target_bytes]
    repeat_count = ((target_bytes - len(prefix)) // len(chunk)) + 1
    return (prefix + chunk * repeat_count)[:target_bytes]


def make_cas_content(seed: int, content_bytes: int, tool_models: list[str]) -> dict[str, Any]:
    tool_model = tool_models[seed % len(tool_models)]
    if "::" in tool_model:
        tool, model = tool_model.split("::", 1)
    else:
        tool, model = tool_model, ""

    return {
        "agent_id": {
            "tool": tool,
            "id": f"bench-agent-{seed % 1000}",
            "model": model,
        },
        "human_author": f"bench-user-{seed % 1000}@example.com",
        "messages": [
            {
                "role": "user",
                "content": make_prompt_text(seed, content_bytes),
            },
            {
                "role": "assistant",
                "content": {
                    "summary": f"generated benchmark response {seed}",
                    "changed_files": 1 + (seed % 8),
                },
            },
        ],
        "total_additions": 10 + (seed % 200),
        "total_deletions": seed % 30,
    }


def make_cas_object(seed: int, content_bytes: int, tool_models: list[str]) -> dict[str, Any]:
    content = make_cas_content(seed, content_bytes, tool_models)
    content_hash = sha256_hex(canonical_json_string(content))
    return {
        "content": content,
        "hash": content_hash,
        "metadata": {
            "benchmark": "cas_upload",
            "seed": str(seed),
        },
    }


def make_cas_batch(
    start_seed: int,
    count: int,
    *,
    content_bytes: int,
    tool_models: list[str],
) -> dict[str, Any]:
    return {
        "objects": [
            make_cas_object(
                start_seed + offset,
                content_bytes,
                tool_models,
            )
            for offset in range(count)
        ]
    }


def validate_cas_response(expected_objects: int):
    def validate(parsed: Any) -> str | None:
        if not isinstance(parsed, dict):
            return "CAS upload response is not an object"
        success_count = parsed.get("success_count")
        failure_count = parsed.get("failure_count")
        if failure_count != 0:
            return f"CAS upload returned failure_count={failure_count}; response={parsed}"
        if success_count != expected_objects:
            return f"CAS upload success_count={success_count}, expected {expected_objects}"
        return None

    return validate


def main() -> None:
    args = parse_args()
    if args.objects_per_request > MAX_CAS_UPLOAD_OBJECTS:
        raise SystemExit(f"--objects-per-request must be <= {MAX_CAS_UPLOAD_OBJECTS}")

    api_keys = require_api_keys(args.api_keys, args.api_key)
    base_url = normalize_base_url(args.base_url)
    url = build_url(base_url, "/worker/cas/upload")
    tool_models = parse_tool_models(args.tool_models)

    def work(index: int):
        api_key = api_keys[index % len(api_keys)]
        headers = api_headers(api_key, args.distinct_id)
        start_seed = args.start_seed + index * args.objects_per_request
        payload = make_cas_batch(
            start_seed,
            args.objects_per_request,
            content_bytes=args.content_bytes,
            tool_models=tool_models,
        )
        return timed_json_request(
            "cas_upload",
            lambda: http_request(
                "POST",
                url,
                headers=headers,
                payload=payload,
                timeout_s=args.timeout,
            ),
            validate=validate_cas_response(args.objects_per_request),
        )

    results, elapsed_s = run_concurrent(args.requests, args.concurrency, work)
    total_objects = args.requests * args.objects_per_request
    print(f"generated_cas_objects={total_objects} objects_per_request={args.objects_per_request}")
    print_summaries(
        f"enterprise CAS upload benchmark elapsed_s={elapsed_s:.2f}",
        summarize(results, elapsed_s),
    )

    if args.allow_errors and args.error_samples > 0:
        print_sample_errors(results, args.error_samples)

    if not args.allow_errors:
        exit_if_failed(results)


if __name__ == "__main__":
    main()
