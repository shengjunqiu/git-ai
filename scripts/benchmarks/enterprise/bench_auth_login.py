#!/usr/bin/env python3
"""Benchmark enterprise auth registration, web login, and OAuth device flow."""

from __future__ import annotations

import argparse
import json
import os
import time
from collections import Counter
from concurrent.futures import FIRST_COMPLETED, ThreadPoolExecutor, wait
from typing import Any

from _common import (
    RequestResult,
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
    timed_json_request,
)


DEVICE_CODE_GRANT = "urn:ietf:params:oauth:grant-type:device_code"
TRACKED_STATUSES = (401, 409, 429)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--base-url",
        default=os.environ.get("ENTERPRISE_BASE_URL", "http://127.0.0.1:8080"),
        help="Enterprise server base URL.",
    )
    parser.add_argument(
        "--mode",
        choices=("login", "register", "oauth"),
        default=os.environ.get("BENCH_AUTH_MODE", "login"),
        help="Auth workload to run.",
    )
    parser.add_argument(
        "--requests",
        type=positive_int,
        default=env_int("BENCH_REQUESTS", 200),
        help="Operations to run. OAuth mode performs one device/code and one token call per operation.",
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
        "--allow-errors",
        action="store_true",
        help="Print error statistics but exit 0 even when requests fail.",
    )

    parser.add_argument(
        "--login-email",
        default=os.environ.get("BENCH_LOGIN_EMAIL"),
        help="Existing user email for --mode login. Defaults to BENCH_LOGIN_EMAIL.",
    )
    parser.add_argument(
        "--login-password",
        default=os.environ.get(
            "BENCH_LOGIN_PASSWORD",
            os.environ.get("BENCH_PASSWORD", "correct-horse-battery"),
        ),
        help="Existing user password for --mode login.",
    )

    parser.add_argument(
        "--email-domain",
        default=os.environ.get("BENCH_EMAIL_DOMAIN", "example.com"),
        help="Verified email domain used by --mode register.",
    )
    parser.add_argument(
        "--email-prefix",
        default=os.environ.get("BENCH_EMAIL_PREFIX", "bench-user"),
        help="Local-part prefix for generated registration emails.",
    )
    parser.add_argument(
        "--run-id",
        default=os.environ.get("BENCH_RUN_ID"),
        help="Stable suffix for generated registration emails. Defaults to current epoch milliseconds.",
    )
    parser.add_argument(
        "--register-password",
        default=os.environ.get("BENCH_PASSWORD", "correct-horse-battery"),
        help="Password used by --mode register.",
    )
    parser.add_argument(
        "--org-id",
        default=os.environ.get("BENCH_ORG_ID"),
        help="Organization UUID for --mode register.",
    )
    parser.add_argument(
        "--org-slug",
        default=os.environ.get("BENCH_ORG_SLUG"),
        help="Organization slug for --mode register.",
    )
    parser.add_argument(
        "--department-id",
        default=os.environ.get("BENCH_DEPARTMENT_ID"),
        help="Department UUID for --mode register.",
    )
    parser.add_argument(
        "--department-slug",
        default=os.environ.get("BENCH_DEPARTMENT_SLUG"),
        help="Department slug for --mode register.",
    )

    parser.add_argument(
        "--client-id",
        default=os.environ.get("BENCH_OAUTH_CLIENT_ID", "git-ai-cli"),
        help="OAuth client_id for --mode oauth.",
    )
    return parser.parse_args()


def require_login_config(args: argparse.Namespace) -> None:
    if not args.login_email:
        raise SystemExit("--login-email or BENCH_LOGIN_EMAIL is required for --mode login")


def require_register_config(args: argparse.Namespace) -> None:
    if not (args.org_id or args.org_slug):
        raise SystemExit("--org-id/--org-slug or BENCH_ORG_ID/BENCH_ORG_SLUG is required")
    if not (args.department_id or args.department_slug):
        raise SystemExit(
            "--department-id/--department-slug or BENCH_DEPARTMENT_ID/BENCH_DEPARTMENT_SLUG is required"
        )


def unique_email(args: argparse.Namespace, index: int) -> str:
    run_id = args.run_id or str(int(time.time() * 1000))
    local_part = f"{args.email_prefix}+{run_id}-{index}"
    return f"{local_part}@{args.email_domain}"


def register_payload(args: argparse.Namespace, index: int) -> dict[str, Any]:
    email = unique_email(args, index)
    payload: dict[str, Any] = {
        "email": email,
        "name": f"Bench User {index}",
        "password": args.register_password,
        "confirm_password": args.register_password,
    }
    if args.org_id:
        payload["org_id"] = args.org_id
    else:
        payload["org_slug"] = args.org_slug

    if args.department_id:
        payload["department_id"] = args.department_id
    else:
        payload["department_slug"] = args.department_slug

    return payload


def validate_login_response(parsed: Any) -> str | None:
    if not isinstance(parsed, dict) or not isinstance(parsed.get("user"), dict):
        return "login response does not include user object"
    return None


def validate_register_response(parsed: Any) -> str | None:
    if not isinstance(parsed, dict) or not isinstance(parsed.get("user"), dict):
        return "register response does not include user object"
    return None


def run_login(args: argparse.Namespace, base_url: str) -> tuple[list[RequestResult], float]:
    require_login_config(args)
    url = build_url(base_url, "/auth/login")
    headers = api_headers()
    payload = {
        "email": args.login_email,
        "password": args.login_password,
    }

    def work(_: int) -> RequestResult:
        return timed_json_request(
            "auth_login",
            lambda: http_request(
                "POST",
                url,
                headers=headers,
                payload=payload,
                timeout_s=args.timeout,
            ),
            validate=validate_login_response,
        )

    return run_concurrent(args.requests, args.concurrency, work)


def run_register(args: argparse.Namespace, base_url: str) -> tuple[list[RequestResult], float]:
    require_register_config(args)
    url = build_url(base_url, "/auth/register")
    headers = api_headers()

    def work(index: int) -> RequestResult:
        return timed_json_request(
            "auth_register",
            lambda: http_request(
                "POST",
                url,
                headers=headers,
                payload=register_payload(args, index),
                timeout_s=args.timeout,
            ),
            validate=validate_register_response,
        )

    return run_concurrent(args.requests, args.concurrency, work)


def timed_oauth_flow(args: argparse.Namespace, base_url: str) -> list[RequestResult]:
    headers = api_headers()
    device_url = build_url(base_url, "/worker/oauth/device/code")
    token_url = build_url(base_url, "/worker/oauth/token")
    results: list[RequestResult] = []

    start = time.perf_counter()
    try:
        status, body = http_request(
            "POST",
            device_url,
            headers=headers,
            timeout_s=args.timeout,
        )
        duration_ms = (time.perf_counter() - start) * 1000
    except Exception as exc:  # noqa: BLE001 - benchmark scripts must capture failures.
        duration_ms = (time.perf_counter() - start) * 1000
        return [RequestResult("oauth_device_code", False, None, duration_ms, str(exc))]

    if status == 429:
        results.append(
            RequestResult("oauth_device_code_rate_limited", False, status, duration_ms, body[:500])
        )
        return results
    if not 200 <= status < 300:
        results.append(RequestResult("oauth_device_code", False, status, duration_ms, body[:500]))
        return results

    try:
        parsed = json.loads(body) if body else {}
    except json.JSONDecodeError as exc:
        results.append(
            RequestResult("oauth_device_code", False, status, duration_ms, f"invalid JSON: {exc}")
        )
        return results

    device_code = parsed.get("device_code")
    if not isinstance(device_code, str) or not device_code:
        results.append(
            RequestResult("oauth_device_code", False, status, duration_ms, "missing device_code")
        )
        return results

    results.append(RequestResult("oauth_device_code", True, status, duration_ms))

    token_payload = {
        "grant_type": DEVICE_CODE_GRANT,
        "device_code": device_code,
        "client_id": args.client_id,
    }
    start = time.perf_counter()
    try:
        status, body = http_request(
            "POST",
            token_url,
            headers=headers,
            payload=token_payload,
            timeout_s=args.timeout,
        )
        duration_ms = (time.perf_counter() - start) * 1000
    except Exception as exc:  # noqa: BLE001 - benchmark scripts must capture failures.
        duration_ms = (time.perf_counter() - start) * 1000
        results.append(RequestResult("oauth_token", False, None, duration_ms, str(exc)))
        return results

    if status == 429:
        results.append(
            RequestResult("oauth_token_rate_limited", False, status, duration_ms, body[:500])
        )
        return results
    if 200 <= status < 300:
        results.append(RequestResult("oauth_token_authorized", True, status, duration_ms))
        return results

    parsed_error = parse_json_object(body)
    error_code = parsed_error.get("error") if parsed_error else None
    if status == 400 and error_code == "authorization_pending":
        results.append(RequestResult("oauth_token_pending", True, status, duration_ms))
        return results

    results.append(RequestResult("oauth_token", False, status, duration_ms, body[:500]))
    return results


def parse_json_object(raw: str) -> dict[str, Any] | None:
    try:
        parsed = json.loads(raw) if raw else {}
    except json.JSONDecodeError:
        return None
    return parsed if isinstance(parsed, dict) else None


def run_oauth(args: argparse.Namespace, base_url: str) -> tuple[list[RequestResult], float]:
    return run_concurrent_flows(
        args.requests,
        args.concurrency,
        lambda _: timed_oauth_flow(args, base_url),
    )


def run_concurrent_flows(
    total_requests: int,
    concurrency: int,
    work: Any,
) -> tuple[list[RequestResult], float]:
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
                results.extend(future.result())
                if next_index < total_requests:
                    pending.add(executor.submit(work, next_index))
                    next_index += 1

    elapsed_s = time.perf_counter() - started_at
    return results, elapsed_s


def print_status_counts(results: list[RequestResult]) -> None:
    counts = Counter(result.status for result in results)
    print("\nHTTP status counts")
    print("status,count")
    for status, count in sorted(counts.items(), key=lambda item: (-1 if item[0] is None else item[0])):
        label = "exception" if status is None else str(status)
        print(f"{label},{count}")

    print("\nTracked auth status counts")
    print("status,count")
    for status in TRACKED_STATUSES:
        print(f"{status},{counts.get(status, 0)}")


def main() -> None:
    args = parse_args()
    base_url = normalize_base_url(args.base_url)

    if args.mode == "login":
        results, elapsed_s = run_login(args, base_url)
        title = f"enterprise auth login benchmark elapsed_s={elapsed_s:.2f}"
    elif args.mode == "register":
        results, elapsed_s = run_register(args, base_url)
        title = f"enterprise auth registration benchmark elapsed_s={elapsed_s:.2f}"
    else:
        results, elapsed_s = run_oauth(args, base_url)
        title = (
            f"enterprise oauth device-flow benchmark "
            f"flows={args.requests} http_results={len(results)} elapsed_s={elapsed_s:.2f}"
        )

    print_summaries(title, summarize(results, elapsed_s))
    print_status_counts(results)

    if not args.allow_errors:
        exit_if_failed(results)


if __name__ == "__main__":
    main()
