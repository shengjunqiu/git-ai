# Enterprise Server Benchmarks

这些脚本用于把 enterprise-server 的手工压测变成可重复执行的命令。脚本只依赖 Python 3 标准库，不需要额外安装压测工具。

## 环境变量

```bash
export ENTERPRISE_BASE_URL=http://127.0.0.1:8080
export ENTERPRISE_API_KEY=your-api-key
export BENCH_CONCURRENCY=20
export BENCH_REQUESTS=1000
```

常用可选变量：

```bash
export BENCH_BATCH_SIZE=100
export BENCH_TIMEOUT_SECONDS=30
export BENCH_DAYS=30
export BENCH_REPO_COUNT=1000
export BENCH_AUTHOR_COUNT=1000
export BENCH_TOOL_MODELS=codex::gpt-5,cursor::claude-4,copilot::gpt-4.1
```

## 快速检查

健康检查和 readiness 不需要 API key：

```bash
python3 scripts/benchmarks/enterprise/bench_health_ready.py
```

脚本输出 CSV 形态的摘要，包含 `rps`、`p95_ms`、`p99_ms` 和 `error_rate_pct`。只要出现 HTTP 错误、超时或非预期响应，脚本会以非 0 退出码结束。

## 造数

通过真实 `/worker/metrics/upload` 接口写入 PosEncoded committed metrics。单请求 batch size 不能超过服务端限制 500。

```bash
ENTERPRISE_API_KEY=... \
python3 scripts/benchmarks/enterprise/seed_metrics.py \
  --events 100000 \
  --batch-size 500 \
  --concurrency 10
```

生成 100 万数据：

```bash
ENTERPRISE_API_KEY=... \
python3 scripts/benchmarks/enterprise/seed_metrics.py \
  --events 1000000 \
  --batch-size 500 \
  --concurrency 20
```

## Metrics 上传压测

```bash
ENTERPRISE_API_KEY=... \
python3 scripts/benchmarks/enterprise/bench_metrics_upload.py \
  --requests 1000 \
  --batch-size 100 \
  --concurrency 20
```

该脚本每个请求都会构造新的 committed event batch，并检查服务端返回的 `errors` 数组。如果出现 partial success，也会按失败处理。

## Dashboard 压测

```bash
ENTERPRISE_API_KEY=... \
python3 scripts/benchmarks/enterprise/bench_dashboard.py \
  --requests 300 \
  --concurrency 20 \
  --days 30
```

覆盖接口：

- `/api/v1/aggregate/summary`
- `/api/v1/aggregate/trends?metric=ai_lines&granularity=day`
- `/api/v1/aggregate/trends?metric=ai_ratio&granularity=week`
- `/api/v1/aggregate/tools`

可以用 `BENCH_ORG` 或 `--org` 指定组织 slug。对比 rollup 前后性能时，分别在服务端设置 `DASHBOARD_USE_ROLLUPS=false` 和 `DASHBOARD_USE_ROLLUPS=true` 后运行同一组命令。
