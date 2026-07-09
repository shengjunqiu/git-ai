#!/usr/bin/env python3
"""Benchmark /api/v1/reports with generated report documents."""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
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
        default=env_int("BENCH_REQUESTS", 100),
        help="Number of report upload requests.",
    )
    parser.add_argument(
        "--commit-count",
        type=positive_int,
        default=env_int("BENCH_REPORT_COMMIT_COUNT", 100),
        help="Commits per report document.",
    )
    parser.add_argument(
        "--project-count",
        type=positive_int,
        default=env_int("BENCH_REPORT_PROJECT_COUNT", 1000),
        help="Number of generated projects to rotate through.",
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
        default=env_float("BENCH_TIMEOUT_SECONDS", 60.0),
        help="Per-request timeout in seconds.",
    )
    parser.add_argument(
        "--days",
        type=positive_int,
        default=env_int("BENCH_DAYS", 30),
        help="Spread generated commit timestamps across this many days.",
    )
    parser.add_argument(
        "--author-count",
        type=positive_int,
        default=env_int("BENCH_AUTHOR_COUNT", 100),
        help="Number of synthetic commit authors.",
    )
    parser.add_argument(
        "--tool-models",
        default=os.environ.get("BENCH_TOOL_MODELS"),
        help="Comma-separated tool::model values.",
    )
    parser.add_argument(
        "--distinct-id",
        default=os.environ.get("BENCH_DISTINCT_ID", "enterprise-bench-report"),
        help="X-Distinct-ID request header.",
    )
    parser.add_argument(
        "--start-seed",
        type=int,
        default=env_int("BENCH_START_SEED", int(time.time())),
        help="Seed offset for generated report content.",
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


def sha1_hex(value: str) -> str:
    return hashlib.sha1(value.encode("utf-8")).hexdigest()


def sha256_hex(value: str) -> str:
    return hashlib.sha256(value.encode("utf-8")).hexdigest()


def utc_rfc3339(timestamp: dt.datetime) -> str:
    return timestamp.replace(microsecond=0).isoformat().replace("+00:00", "Z")


def commit_stats(seed: int) -> dict[str, int]:
    human = 5 + (seed % 45)
    ai = 10 + (seed % 80)
    mixed = seed % 11
    deleted = seed % 20
    accepted = max(0, ai - (seed % 7))
    return {
        "git_diff_added_lines": human + ai + mixed,
        "git_diff_deleted_lines": deleted,
        "ai_additions": ai,
        "human_additions": human,
        "mixed_additions": mixed,
        "unknown_additions": 0,
        "ai_accepted": accepted,
        "total_ai_additions": ai,
        "total_ai_deletions": seed % 9,
        "time_waiting_for_ai": 100 + (seed % 600),
    }


def make_commit(seed: int, *, now: dt.datetime, days: int, author_count: int) -> dict[str, Any]:
    seconds = seed % max(1, days * 24 * 60 * 60)
    author_time = utc_rfc3339(now - dt.timedelta(seconds=seconds))
    return {
        "sha": sha1_hex(f"bench-report-commit-{seed}"),
        "author": f"Bench Author {seed % author_count} <bench-{seed % author_count}@example.com>",
        "author_time": author_time,
        "subject": f"benchmark report commit {seed}",
        "has_authorship_note": True,
        "stats": commit_stats(seed),
    }


def add_stats(target: dict[str, int], stats: dict[str, int]) -> None:
    for key, value in stats.items():
        target[key] = target.get(key, 0) + value


def make_tool_model_breakdown(commits: list[dict[str, Any]], tool_models: list[str]) -> dict[str, Any]:
    breakdown: dict[str, dict[str, int]] = {}
    for index, commit in enumerate(commits):
        tool_model = tool_models[index % len(tool_models)]
        stats = commit["stats"]
        item = breakdown.setdefault(
            tool_model,
            {
                "ai_additions": 0,
                "human_additions": 0,
                "mixed_additions": 0,
                "total_ai_additions": 0,
                "total_ai_deletions": 0,
                "ai_accepted": 0,
                "time_waiting_for_ai": 0,
            },
        )
        add_stats(item, stats)
    return breakdown


def make_report(
    seed: int,
    *,
    commit_count: int,
    project_count: int,
    now: dt.datetime,
    days: int,
    author_count: int,
    tool_models: list[str],
) -> dict[str, Any]:
    project_index = seed % project_count
    commit_start_seed = seed * commit_count
    commits = [
        make_commit(
            commit_start_seed + offset,
            now=now,
            days=days,
            author_count=author_count,
        )
        for offset in range(commit_count)
    ]

    summary: dict[str, int] = {}
    for commit in commits:
        add_stats(summary, commit["stats"])

    total_additions = max(1, summary.get("git_diff_added_lines", 0))
    ratios = {
        "ai": summary.get("ai_additions", 0) / total_additions,
        "human": summary.get("human_additions", 0) / total_additions,
        "mixed": summary.get("mixed_additions", 0) / total_additions,
        "unknown": summary.get("unknown_additions", 0) / total_additions,
    }

    generated_at = utc_rfc3339(now)
    return {
        "schema_version": "git-ai-report/1.0.0",
        "generated_at": generated_at,
        "tool_version": "1.3.2-bench",
        "repo": {
            "workdir": f"/benchmark/repo-{project_index}",
            "remote_url_hash": sha256_hex(f"https://example.com/bench/report-{project_index}.git"),
            "branch": f"bench-branch-{project_index % 8}",
            "head_commit": commits[-1]["sha"] if commits else None,
        },
        "range": {
            "mode": "benchmark",
            "from": commits[0]["sha"] if commits else None,
            "to": commits[-1]["sha"] if commits else None,
            "since": utc_rfc3339(now - dt.timedelta(days=days)),
            "until": generated_at,
            "commit_count": commit_count,
            "commits_with_authorship": commit_count,
            "commits_without_authorship": 0,
        },
        "summary": summary,
        "ratios": ratios,
        "tool_model_breakdown": make_tool_model_breakdown(commits, tool_models),
        "commits": commits,
    }


def validate_report_response(expected_commits: int):
    def validate(parsed: Any) -> str | None:
        if not isinstance(parsed, dict):
            return "report upload response is not an object"
        if "project_id" not in parsed or "upload_id" not in parsed:
            return f"report upload response missing ids; response={parsed}"

        inserted = parsed.get("inserted_commits")
        updated = parsed.get("updated_commits", parsed.get("duplicate_commits", 0))
        if not isinstance(inserted, int) or not isinstance(updated, int):
            return f"report upload response missing commit counts; response={parsed}"
        if inserted + updated != expected_commits:
            return (
                f"report upload inserted+updated={inserted + updated}, "
                f"expected {expected_commits}; response={parsed}"
            )
        return None

    return validate


def main() -> None:
    args = parse_args()
    api_keys = require_api_keys(args.api_keys, args.api_key)
    base_url = normalize_base_url(args.base_url)
    url = build_url(base_url, "/api/v1/reports")
    now = dt.datetime.now(dt.UTC)
    tool_models = parse_tool_models(args.tool_models)

    def work(index: int):
        api_key = api_keys[index % len(api_keys)]
        headers = api_headers(api_key, args.distinct_id)
        report_seed = args.start_seed + index
        payload = make_report(
            report_seed,
            commit_count=args.commit_count,
            project_count=args.project_count,
            now=now,
            days=args.days,
            author_count=args.author_count,
            tool_models=tool_models,
        )
        return timed_json_request(
            "report_upload",
            lambda: http_request(
                "POST",
                url,
                headers=headers,
                payload=payload,
                timeout_s=args.timeout,
            ),
            validate=validate_report_response(args.commit_count),
        )

    results, elapsed_s = run_concurrent(args.requests, args.concurrency, work)
    total_commits = args.requests * args.commit_count
    print(f"generated_report_commits={total_commits} commit_count={args.commit_count}")
    print_summaries(
        f"enterprise report upload benchmark elapsed_s={elapsed_s:.2f}",
        summarize(results, elapsed_s),
    )

    if args.allow_errors and args.error_samples > 0:
        print_sample_errors(results, args.error_samples)

    if not args.allow_errors:
        exit_if_failed(results)


if __name__ == "__main__":
    main()
