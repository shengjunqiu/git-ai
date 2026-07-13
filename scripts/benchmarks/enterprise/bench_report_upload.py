#!/usr/bin/env python3
"""Benchmark /api/v1/reports with generated report documents.
对 /api/v1/reports 接口进行压测，使用自动生成的报告文档。"""

from __future__ import annotations

// 测试

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
        "--requests",
        type=positive_int,
        default=env_int("BENCH_REQUESTS", 100),
        help="报告上传请求的总次数。",
    )
    parser.add_argument(
        "--commit-count",
        type=positive_int,
        default=env_int("BENCH_REPORT_COMMIT_COUNT", 100),
        help="每个报告文档中包含的 commit 数量。",
    )
    parser.add_argument(
        "--project-count",
        type=positive_int,
        default=env_int("BENCH_REPORT_PROJECT_COUNT", 1000),
        help="生成的虚拟项目数量，请求会在这些项目中轮转。",
    )
    parser.add_argument(
        "--concurrency",
        type=positive_int,
        default=env_int("BENCH_CONCURRENCY", 20),
        help="并发工作线程数。",
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
        help="生成的 commit 时间戳跨越的天数范围。",
    )
    parser.add_argument(
        "--author-count",
        type=positive_int,
        default=env_int("BENCH_AUTHOR_COUNT", 100),
        help="生成的虚拟 commit 作者数量。",
    )
    parser.add_argument(
        "--tool-models",
        default=os.environ.get("BENCH_TOOL_MODELS"),
        help="逗号分隔的 tool::model 值，用于生成 tool/model 维度的统计分解。",
    )
    parser.add_argument(
        "--distinct-id",
        default=os.environ.get("BENCH_DISTINCT_ID", "enterprise-bench-report"),
        help="请求头 X-Distinct-ID 的值。",
    )
    parser.add_argument(
        "--start-seed",
        type=int,
        default=env_int("BENCH_START_SEED", int(time.time())),
        help="生成报告内容时使用的种子偏移量，用于保证每次运行的数据唯一性。",
    )
    parser.add_argument(
        "--allow-errors",
        action="store_true",
        help="即使请求失败也打印错误统计并以退出码 0 结束（不抛异常）。",
    )
    parser.add_argument(
        "--error-samples",
        type=int,
        default=env_int("BENCH_ERROR_SAMPLES", 5),
        help="发生错误时打印的示例错误数量，设为 0 则禁用。",
    )
    return parser.parse_args()


# ---------------------------------------------------------------------------
# 哈希工具函数
# ---------------------------------------------------------------------------


def sha1_hex(value: str) -> str:
    """对字符串计算 SHA-1 哈希并返回十六进制字符串。"""
    return hashlib.sha1(value.encode("utf-8")).hexdigest()


def sha256_hex(value: str) -> str:
    """对字符串计算 SHA-256 哈希并返回十六进制字符串。"""
    return hashlib.sha256(value.encode("utf-8")).hexdigest()


# ---------------------------------------------------------------------------
# 时间格式化
# ---------------------------------------------------------------------------


def utc_rfc3339(timestamp: dt.datetime) -> str:
    """将 datetime 对象格式化为 UTC RFC 3339 字符串（如 "2024-01-01T00:00:00Z"）。"""
    return timestamp.replace(microsecond=0).isoformat().replace("+00:00", "Z")


# ---------------------------------------------------------------------------
# 数据生成器 —— 根据 seed 生成确定性的测试数据
# ---------------------------------------------------------------------------


def commit_stats(seed: int) -> dict[str, int]:
    """根据 seed 生成一个 commit 的统计指标（代码行数变化等）。

    所有值由 seed 确定性派生，目的是模拟真实数据分布：
    - human: 人工编写的行数
    - ai: AI 新增的行数
    - mixed: 人机混合编写的行数
    - deleted: 删除的行数
    - accepted: 最终接受的 AI 行数（ai 减去一小部分）
    """
    human = 5 + (seed % 45)          # 5 ~ 49 行
    ai = 10 + (seed % 80)            # 10 ~ 89 行
    mixed = seed % 11                # 0 ~ 10 行
    deleted = seed % 20              # 0 ~ 19 行
    accepted = max(0, ai - (seed % 7))  # AI 接受量，略小于 ai
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
        "time_waiting_for_ai": 100 + (seed % 600),  # 模拟等待 AI 的耗时（秒）
    }


def make_commit(seed: int, *, now: dt.datetime, days: int, author_count: int) -> dict[str, Any]:
    """根据 seed 生成一条虚拟 commit 记录。

    参数：
        seed:         用于确定性地生成各字段值的种子。
        now:          基准时间，commit 时间相对于此时间向历史偏移。
        days:         时间偏移的最大天数。
        author_count: 虚拟作者总数，用于按 seed 轮转作者。
    """
    seconds = seed % max(1, days * 24 * 60 * 60)                # 将 seed 映射到 [0, days) 天的秒数范围
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
    """将 stats 中的各指标累加到 target 字典中（原地修改）。"""
    for key, value in stats.items():
        target[key] = target.get(key, 0) + value


def make_tool_model_breakdown(commits: list[dict[str, Any]], tool_models: list[str]) -> dict[str, Any]:
    """按 tool::model 维度对所有 commit 的统计数据进行汇总分解。

    每个 commit 按轮转顺序分配一个 tool_model，然后按 tool_model 聚合各指标。
    """
    breakdown: dict[str, dict[str, int]] = {}
    for index, commit in enumerate(commits):
        tool_model = tool_models[index % len(tool_models)]      # 轮转分配
        stats = commit["stats"]
        # 如果该 tool_model 还没初始化，先写入初始值
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
        add_stats(item, stats)                                  # 累加到对应 tool_model 的汇总中
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
    """根据 seed 生成一份完整的虚拟报告文档。

    参数：
        seed:          报告种子，由此派生项目归属、各 commit 的种子等。
        commit_count:  报告中的 commit 数量。
        project_count: 虚拟项目总数，用于按 seed 轮转选择项目。
        now:           基准时间。
        days:          时间跨度（天）。
        author_count:  虚拟作者数量。
        tool_models:   tool::model 列表，用于生成工具/模型维度的分解。

    返回：
        一份符合 git-ai-report/1.0.0 schema 的报告字典。
    """
    project_index = seed % project_count                         # 按 seed 选择一个虚拟项目
    commit_start_seed = seed * commit_count                      # 每份报告的 commit 起始 seed
    # 生成该报告包含的所有 commit
    commits = [
        make_commit(
            commit_start_seed + offset,
            now=now,
            days=days,
            author_count=author_count,
        )
        for offset in range(commit_count)
    ]

    # 汇总所有 commit 的统计数据
    summary: dict[str, int] = {}
    for commit in commits:
        add_stats(summary, commit["stats"])

    # 计算各类代码行数的占比
    total_additions = max(1, summary.get("git_diff_added_lines", 0))  # 避免除零
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
        # 仓库信息
        "repo": {
            "workdir": f"/benchmark/repo-{project_index}",
            "remote_url_hash": sha256_hex(f"https://example.com/bench/report-{project_index}.git"),
            "branch": f"bench-branch-{project_index % 8}",
            "head_commit": commits[-1]["sha"] if commits else None,
        },
        # 分析范围信息
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


# ---------------------------------------------------------------------------
# 响应校验
# ---------------------------------------------------------------------------


def validate_report_response(expected_commits: int):
    """返回一个校验函数，用于验证 API 返回的报告上传响应是否合法。

    参数：
        expected_commits: 期望的 commit 总数（新增 + 更新）。

    返回的校验函数会在响应不合法时返回错误描述字符串，合法则返回 None。
    """
    def validate(parsed: Any) -> str | None:
        # 响应必须是字典
        if not isinstance(parsed, dict):
            return "report upload response is not an object"
        # 必须包含 project_id 和 upload_id
        if "project_id" not in parsed or "upload_id" not in parsed:
            return f"report upload response missing ids; response={parsed}"

        inserted = parsed.get("inserted_commits")
        updated = parsed.get("updated_commits", parsed.get("duplicate_commits", 0))
        # 提交计数必须为整数
        if not isinstance(inserted, int) or not isinstance(updated, int):
            return f"report upload response missing commit counts; response={parsed}"
        # 新增 + 更新 的总数应等于期望的 commit 数
        if inserted + updated != expected_commits:
            return (
                f"report upload inserted+updated={inserted + updated}, "
                f"expected {expected_commits}; response={parsed}"
            )
        return None

    return validate


# ---------------------------------------------------------------------------
# 主入口
# ---------------------------------------------------------------------------


def main() -> None:
    """主函数：解析参数 → 生成报告 → 并发上传 → 打印汇总结果。"""
    args = parse_args()
    # 获取 API 密钥列表（支持单密钥或多密钥）
    api_keys = require_api_keys(args.api_keys, args.api_key)
    base_url = normalize_base_url(args.base_url)
    url = build_url(base_url, "/api/v1/reports")
    now = dt.datetime.now(dt.UTC)
    # 解析 tool::model 列表
    tool_models = parse_tool_models(args.tool_models)

    def work(index: int):
        """单个并发工作单元：生成报告、发送 POST 请求、计时并校验响应。"""
        api_key = api_keys[index % len(api_keys)]               # 轮转使用多个 API 密钥
        headers = api_headers(api_key, args.distinct_id)
        report_seed = args.start_seed + index                   # 每个请求使用不同的种子
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

    # 并发执行所有请求
    results, elapsed_s = run_concurrent(args.requests, args.concurrency, work)
    total_commits = args.requests * args.commit_count
    print(f"generated_report_commits={total_commits} commit_count={args.commit_count}")
    # 打印汇总的性能统计
    print_summaries(
        f"enterprise report upload benchmark elapsed_s={elapsed_s:.2f}",
        summarize(results, elapsed_s),
    )

    # 如果允许错误且配置了打印错误样例，则输出示例错误
    if args.allow_errors and args.error_samples > 0:
        print_sample_errors(results, args.error_samples)

    # 如果不允许错误，则检查并退出（有失败则返回非零退出码）
    if not args.allow_errors:
        exit_if_failed(results)


if __name__ == "__main__":
    main()
