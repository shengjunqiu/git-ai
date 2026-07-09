#!/usr/bin/env python3
"""Benchmark enterprise auth registration, web login, and OAuth device flow."""

from __future__ import annotations

import argparse
import csv
import ipaddress
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
    print_sample_errors,
    print_summaries,
    run_concurrent,
    summarize,
    timed_json_request,
)


DEVICE_CODE_GRANT = "urn:ietf:params:oauth:grant-type:device_code"
TRACKED_STATUSES = (401, 409, 429, 500)


def client_ip_mode(value: str) -> str:
    normalized = value.strip().lower()
    if normalized not in {"none", "same", "unique", "pool"}:
        raise argparse.ArgumentTypeError("client IP mode must be none, same, unique, or pool")
    return normalized


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
        "--error-samples",
        type=int,
        default=env_int("BENCH_ERROR_SAMPLES", 5),
        help="Number of sample errors to print when errors occur. Use 0 to disable.",
    )
    parser.add_argument(
        "--client-ip-mode",
        type=client_ip_mode,
        default=client_ip_mode(os.environ.get("BENCH_CLIENT_IP_MODE", "none")),
        help=(
            "How to set X-Forwarded-For for rate-limit tests: none preserves existing behavior, "
            "same simulates one IP, unique simulates one IP per operation, pool rotates a fixed IP pool."
        ),
    )
    parser.add_argument(
        "--client-ip-base",
        default=os.environ.get("BENCH_CLIENT_IP_BASE", "10.72.0.1"),
        help="Base IPv4 address used by --client-ip-mode same, unique, or pool.",
    )
    parser.add_argument(
        "--client-ip-pool-size",
        type=positive_int,
        default=env_int("BENCH_CLIENT_IP_POOL_SIZE", 256),
        help="Number of IPs to rotate when --client-ip-mode pool is used.",
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
        "--login-users-file",
        default=os.environ.get("BENCH_LOGIN_USERS_FILE"),
        help="CSV file for --mode login with email,password columns. Overrides single login email.",
    )
    parser.add_argument(
        "--login-user-count",
        type=int,
        default=env_int("BENCH_LOGIN_USER_COUNT", 0),
        help=(
            "Generate this many login users using --login-email-prefix, --login-run-id, "
            "and --login-email-domain. Useful after seeding users with --mode register."
        ),
    )
    parser.add_argument(
        "--login-email-domain",
        default=os.environ.get("BENCH_LOGIN_EMAIL_DOMAIN"),
        help="Email domain for generated login users. Defaults to --email-domain.",
    )
    parser.add_argument(
        "--login-email-prefix",
        default=os.environ.get("BENCH_LOGIN_EMAIL_PREFIX"),
        help="Local-part prefix for generated login users. Defaults to --email-prefix.",
    )
    parser.add_argument(
        "--login-run-id",
        default=os.environ.get("BENCH_LOGIN_RUN_ID"),
        help="Run id for generated login users. Defaults to --run-id.",
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
    if args.login_user_count < 0:
        raise SystemExit("--login-user-count cannot be negative")
    if args.login_users_file:
        return
    if args.login_user_count:
        return
    if not args.login_email:
        raise SystemExit("--login-email or BENCH_LOGIN_EMAIL is required for --mode login")


def require_register_config(args: argparse.Namespace) -> None:
    if not (args.org_id or args.org_slug):
        raise SystemExit("--org-id/--org-slug or BENCH_ORG_ID/BENCH_ORG_SLUG is required")
    if not (args.department_id or args.department_slug):
        raise SystemExit(
            "--department-id/--department-slug or BENCH_DEPARTMENT_ID/BENCH_DEPARTMENT_SLUG is required"
        )


def request_headers(args: argparse.Namespace, index: int) -> dict[str, str]:
    headers = api_headers()
    client_ip = client_ip_for_index(args, index)
    if client_ip:
        headers["X-Forwarded-For"] = client_ip
    return headers


def client_ip_for_index(args: argparse.Namespace, index: int) -> str | None:
    if args.client_ip_mode == "none":
        return None

    try:
        base_ip = ipaddress.IPv4Address(args.client_ip_base)
    except ipaddress.AddressValueError as exc:
        raise SystemExit(f"--client-ip-base must be a valid IPv4 address: {exc}") from exc

    if args.client_ip_mode == "same":
        offset = 0
    elif args.client_ip_mode == "unique":
        offset = index
    else:
        offset = index % args.client_ip_pool_size

    try:
        return str(base_ip + offset)
    except ipaddress.AddressValueError as exc:
        raise SystemExit(
            "--client-ip-base plus generated offset exceeds the IPv4 address range"
        ) from exc


def load_login_credentials(args: argparse.Namespace) -> list[tuple[str, str]]:
    require_login_config(args)
    if not args.login_users_file:
        if args.login_user_count:
            return [
                (generated_login_email(args, index), args.login_password)
                for index in range(args.login_user_count)
            ]
        return [(args.login_email, args.login_password)]

    credentials: list[tuple[str, str]] = []
    with open(args.login_users_file, newline="", encoding="utf-8") as handle:
        sample = handle.read(4096)
        handle.seek(0)
        try:
            has_header = csv.Sniffer().has_header(sample) if sample.strip() else False
        except csv.Error:
            has_header = False
        if has_header:
            reader = csv.DictReader(handle)
            for row in reader:
                email = (row.get("email") or row.get("login_email") or "").strip()
                password = (row.get("password") or row.get("login_password") or "").strip()
                if email and password:
                    credentials.append((email, password))
        else:
            reader = csv.reader(handle)
            for row in reader:
                if len(row) < 2:
                    continue
                email = row[0].strip()
                password = row[1].strip()
                if email and password:
                    credentials.append((email, password))

    if not credentials:
        raise SystemExit(
            "--login-users-file must contain at least one email,password credential row"
        )
    return credentials


def unique_email(args: argparse.Namespace, index: int) -> str:
    run_id = args.run_id or str(int(time.time() * 1000))
    local_part = f"{args.email_prefix}+{run_id}-{index}"
    return f"{local_part}@{args.email_domain}"


def generated_login_email(args: argparse.Namespace, index: int) -> str:
    run_id = args.login_run_id or args.run_id
    if not run_id:
        raise SystemExit("--login-run-id or --run-id is required with --login-user-count")

    prefix = args.login_email_prefix or args.email_prefix
    domain = args.login_email_domain or args.email_domain
    local_part = f"{prefix}+{run_id}-{index}"
    return f"{local_part}@{domain}"


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
    credentials = load_login_credentials(args)
    url = build_url(base_url, "/auth/login")

    def work(index: int) -> RequestResult:
        email, password = credentials[index % len(credentials)]
        payload = {
            "email": email,
            "password": password,
        }
        return timed_json_request(
            "auth_login",
            lambda: http_request(
                "POST",
                url,
                headers=request_headers(args, index),
                payload=payload,
                timeout_s=args.timeout,
            ),
            validate=validate_login_response,
        )

    return run_concurrent(args.requests, args.concurrency, work)


def run_register(args: argparse.Namespace, base_url: str) -> tuple[list[RequestResult], float]:
    require_register_config(args)
    url = build_url(base_url, "/auth/register")

    def work(index: int) -> RequestResult:
        return timed_json_request(
            "auth_register",
            lambda: http_request(
                "POST",
                url,
                headers=request_headers(args, index),
                payload=register_payload(args, index),
                timeout_s=args.timeout,
            ),
            validate=validate_register_response,
        )

    return run_concurrent(args.requests, args.concurrency, work)


def timed_oauth_flow(args: argparse.Namespace, base_url: str, index: int) -> list[RequestResult]:
    headers = request_headers(args, index)
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
        lambda index: timed_oauth_flow(args, base_url, index),
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

    if args.allow_errors and args.error_samples > 0:
        print_sample_errors(results, args.error_samples)

    if not args.allow_errors:
        exit_if_failed(results)


if __name__ == "__main__":
    main()
