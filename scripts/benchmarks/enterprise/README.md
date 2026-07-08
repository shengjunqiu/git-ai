# Enterprise Server Benchmarks

这些脚本用于把 enterprise-server 的手工压测变成可重复执行的命令。脚本只依赖 Python 3 标准库，不需要额外安装压测工具。

## 环境变量

```bash
export ENTERPRISE_BASE_URL=http://127.0.0.1:8080
export ENTERPRISE_API_KEY=your-api-key
# 可选：高并发造数时轮换多个 key，避免单 key metrics 限流影响压测。
export ENTERPRISE_API_KEYS=key-1,key-2,key-3
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

## 注册、登录和 OAuth 压测

网页登录压测使用已有测试用户：

```bash
BENCH_LOGIN_EMAIL=bench@example.com \
BENCH_LOGIN_PASSWORD=correct-horse-battery \
python3 scripts/benchmarks/enterprise/bench_auth_login.py \
  --mode login \
  --requests 1000 \
  --concurrency 30
```

注册压测会为每个请求生成唯一邮箱，避免重复邮箱冲突干扰结果。邮箱域名必须已经在目标组织里验证通过：

```bash
BENCH_EMAIL_DOMAIN=example.com \
BENCH_ORG_ID=00000000-0000-0000-0000-000000000000 \
BENCH_DEPARTMENT_ID=00000000-0000-0000-0000-000000000000 \
python3 scripts/benchmarks/enterprise/bench_auth_login.py \
  --mode register \
  --requests 500 \
  --concurrency 30
```

也可以用 `BENCH_ORG_SLUG` 和 `BENCH_DEPARTMENT_SLUG` 代替 UUID。OAuth device flow 压测会先请求 device code，再立即请求 token；未授权设备返回的 `authorization_pending` 会被按预期成功分类，429 会单独统计：

```bash
python3 scripts/benchmarks/enterprise/bench_auth_login.py \
  --mode oauth \
  --requests 1000 \
  --concurrency 50
```

该脚本额外输出 HTTP status 统计，并单独列出 401、409、429。默认只要出现非预期错误就以非 0 退出；需要只采集错误率时可加 `--allow-errors`。

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

## Postgres 观测

本地和部署 compose 已配置 `shared_preload_libraries=pg_stat_statements`。修改该配置后需要重启 Postgres，再创建 extension：

```bash
docker compose restart postgres
docker compose exec postgres psql -U gitai -d gitai_enterprise \
  -c "CREATE EXTENSION IF NOT EXISTS pg_stat_statements;"
```

压测后执行：

```bash
docker compose exec -T postgres psql -U gitai -d gitai_enterprise \
  < scripts/benchmarks/enterprise/postgres_observability.sql
```

输出包含 `pg_stat_statements` 配置、按平均耗时排序的慢查询、按总耗时排序的查询，以及当前连接状态。
