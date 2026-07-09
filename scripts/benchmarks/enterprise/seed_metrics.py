#!/usr/bin/env python3
"""Seed large metrics datasets through /worker/metrics/upload.
通过 /worker/metrics/upload 接口批量灌入大规模指标数据。"""

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
    require_api_keys,
    run_concurrent,
    summarize,
    timed_json_request,
)


# ---------------------------------------------------------------------------
# 命令行参数解析
# ---------------------------------------------------------------------------


def parse_args() -> argparse.Namespace:
    """解析命令行参数，支持通过环境变量设置默认值。"""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--base-url",
        default=os.environ.get("ENTERPRISE_BASE_URL", "http://127.0.0.1:8080"),
        help="Enterprise 服务端基础 URL。",
    )
    parser.add_argument(
        "--api-key",
        default=os.environ.get("ENTERPRISE_API_KEY"),
        help="Enterprise API 密钥。默认读取 ENTERPRISE_API_KEY 环境变量。",
    )
    parser.add_argument(
        "--api-keys",
        default=os.environ.get("ENTERPRISE_API_KEYS"),
        help="逗号分隔的多个 API 密钥。默认读取 ENTERPRISE_API_KEYS 环境变量。",
    )
    parser.add_argument(
        "--events",
        type=positive_int,
        default=env_int("SEED_METRICS_EVENTS", 100000),
        help="需要上传的指标事件总数。",
    )
    parser.add_argument(
        "--batch-size",
        type=positive_int,
        default=env_int("BENCH_BATCH_SIZE", MAX_METRICS_BATCH_SIZE),
        help=f"每次上传请求包含的事件数量。服务端限制为 {MAX_METRICS_BATCH_SIZE}。",
    )
    parser.add_argument(
        "--concurrency",
        type=positive_int,
        default=env_int("BENCH_CONCURRENCY", 10),
        help="并发上传工作线程数。",
    )
    parser.add_argument(
        "--timeout",
        type=float,
        default=env_float("BENCH_TIMEOUT_SECONDS", 60.0),
        help="每个请求的超时时间（秒）。",
    )
    parser.add_argument(
        "--days",
        type=positive_int,
        default=env_int("BENCH_DAYS", 30),
        help="生成的事件时间戳跨越的天数范围。",
    )
    parser.add_argument(
        "--repo-count",
        type=positive_int,
        default=env_int("BENCH_REPO_COUNT", 1000),
        help="生成的虚拟仓库数量，事件会在这些仓库间轮转。",
    )
    parser.add_argument(
        "--author-count",
        type=positive_int,
        default=env_int("BENCH_AUTHOR_COUNT", 1000),
        help="生成的虚拟 commit 作者数量。",
    )
    parser.add_argument(
        "--tool-models",
        default=os.environ.get("BENCH_TOOL_MODELS"),
        help="逗号分隔的 tool::model 值。",
    )
    parser.add_argument(
        "--distinct-id",
        default=os.environ.get("BENCH_DISTINCT_ID", "enterprise-bench-seed"),
        help="请求头 X-Distinct-ID 的值。",
    )
    parser.add_argument(
        "--start-seed",
        type=int,
        default=env_int("BENCH_START_SEED", 1),
        help="生成 commit SHA 时使用的种子偏移量。",
    )
    return parser.parse_args()


# ---------------------------------------------------------------------------
# 响应校验
# ---------------------------------------------------------------------------


def validate_metrics_response(parsed) -> str | None:
    """校验 /worker/metrics/upload 接口的响应是否合法。

    如果响应中包含 errors 列表（表示部分事件写入失败），则返回错误描述；
    否则返回 None 表示校验通过。
    """
    errors = parsed.get("errors", []) if isinstance(parsed, dict) else []
    if errors:
        sample = errors[0] if isinstance(errors, list) and errors else {}
        return f"metrics upload returned {len(errors)} event errors; sample={sample}"
    return None


# ---------------------------------------------------------------------------
# 主入口
# ---------------------------------------------------------------------------


def main() -> None:
    """主函数：解析参数 → 分批生成指标 → 并发上传 → 打印汇总结果。"""
    args = parse_args()
    # 检查批次大小是否超过服务端限制
    if args.batch_size > MAX_METRICS_BATCH_SIZE:
        raise SystemExit(f"--batch-size must be <= {MAX_METRICS_BATCH_SIZE}")

    api_keys = require_api_keys(args.api_keys, args.api_key)
    base_url = normalize_base_url(args.base_url)
    url = build_url(base_url, "/worker/metrics/upload")
    tool_models = parse_tool_models(args.tool_models)
    now_s = int(time.time())                                      # 当前 Unix 时间戳（秒），事件时间相对此偏移
    # 计算需要的总批次数（向上取整，确保覆盖所有事件）
    batches = math.ceil(args.events / args.batch_size)

    def work(index: int):
        """单个并发工作单元：生成一批指标事件、发送 POST 请求、计时并校验响应。"""
        api_key = api_keys[index % len(api_keys)]                 # 轮转使用多个 API 密钥
        headers = api_headers(api_key, args.distinct_id)
        start_seed = args.start_seed + index * args.batch_size    # 每批使用不同的种子起始值
        # 最后一批可能不足 batch_size，取剩余事件数
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

    # 并发执行所有批次的请求（并发数以 batch 数而非事件数为单位）
    results, elapsed_s = run_concurrent(batches, args.concurrency, work)
    print(
        f"seeded_events_target={args.events} batches={batches} "
        f"batch_size={args.batch_size}"
    )
    # 打印汇总的性能统计
    print_summaries(
        f"enterprise metrics seed elapsed_s={elapsed_s:.2f}",
        summarize(results, elapsed_s),
    )
    # 如果有请求失败，以非零退出码结束
    exit_if_failed(results)


if __name__ == "__main__":
    main()
