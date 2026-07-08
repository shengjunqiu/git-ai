# Git AI Enterprise Server - 部署指南

## 系统要求

- **操作系统**: Linux (推荐 Ubuntu 22.04+ / CentOS 8+)
- **Docker**: 24.0+ 
- **Docker Compose**: v2.0+
- **内存**: 最低 2GB，推荐 4GB+
- **磁盘**: 最低 10GB
- **端口**: 8080 (API), 5433 (PostgreSQL), 6379 (Redis), 9000/9001 (MinIO)

## 快速部署

### 1. 解压部署包

```bash
tar xzf git-ai-enterprise-server-deploy.tar.gz
cd git-ai-enterprise-server-deploy
```

### 2. 配置环境变量

```bash
cp .env.example .env
# 编辑 .env，至少修改以下项：
#   JWT_SECRET      - 随机密钥，至少 32 字符
#   POSTGRES_PASSWORD - 数据库密码
vi .env
```

### 3. 加载镜像

```bash
docker load -i images/git-ai-enterprise-server-api.tar
```

### 4. 一键部署

```bash
bash scripts/deploy.sh
```

或者手动执行：

```bash
# 启动服务
docker compose up -d

# 等待服务就绪 (约 15 秒)
sleep 15

# 初始化数据库 (仅首次)
bash scripts/migrate.sh --init

# 后续升级
bash scripts/migrate.sh --upgrade
```

### 5. 验证服务

```bash
# 健康检查
curl http://localhost:8080/health

# 预期返回: {"service":"git-ai-enterprise-server","status":"ok","version":"0.1.0"}
```

## 服务端口

| 服务 | 端口 | 说明 |
|------|------|------|
| API | 8080 | REST API 服务 |
| PostgreSQL | 5433 | 数据库 (外部访问) |
| Redis | 6379 | 缓存 |
| MinIO API | 9000 | S3 兼容存储 |
| MinIO Console | 9001 | 存储管理界面 |

## 数据持久化

数据通过 Docker Volume 持久化：

- `pgdata` - PostgreSQL 数据
- `minio_data` - MinIO 对象存储数据

> **重要**: 删除容器不会丢失数据，但 `docker compose down -v` 会清除所有数据！

## 常用运维命令

```bash
# 查看服务状态
docker compose ps

# 查看日志
docker compose logs -f api          # API 日志
docker compose logs -f postgres     # 数据库日志

# 重启服务
docker compose restart api

# 停止所有服务
docker compose down

# 停止并清除数据 (危险!)
docker compose down -v
```

## 升级部署

```bash
# 1. 加载新镜像
docker load -i images/git-ai-enterprise-server-api.tar

# 2. 重启 API 容器
docker compose up -d api

# 3. 执行数据库迁移
bash scripts/migrate.sh --upgrade
```

## 数据库迁移脚本

`migrations/` 目录包含所有 SQL 迁移脚本，按编号顺序执行：

| 编号 | 说明 |
|------|------|
| 001 | 初始 Schema |
| 002 | 仓库访问列表 |
| 003 | 企业功能 |
| 004 | 数据隔离 |
| 005 | Linewell 组织及测试数据 |

PostgreSQL 容器首次启动时会自动执行 `migrations/` 目录中的脚本（通过 `docker-entrypoint-initdb.d`）。
后续迁移需手动执行 `migrate.sh`。

## API Key 管理

API Key 通过数据库管理，创建方式：

```bash
# 进入 PostgreSQL 容器
docker compose exec postgres psql -U gitai -d gitai_enterprise

# 查看现有 Key
SELECT key_prefix, name FROM api_keys WHERE revoked_at IS NULL;
```

建议通过 API 接口创建新的 API Key，以便获取明文密钥。

## Postgres 慢查询观测

部署包的 `docker-compose.yml` 已为 PostgreSQL 启用：

```text
shared_preload_libraries=pg_stat_statements
pg_stat_statements.track=all
```

首次启用或修改该配置后需要重启 PostgreSQL：

```bash
docker compose restart postgres
docker compose restart api
```

在目标数据库中创建 extension：

```bash
docker compose exec postgres psql -U ${POSTGRES_USER:-gitai} -d ${POSTGRES_DB:-gitai_enterprise} \
  -c "CREATE EXTENSION IF NOT EXISTS pg_stat_statements;"
```

压测后查看最慢查询：

```sql
SELECT
    calls,
    ROUND(mean_exec_time::numeric, 2) AS mean_exec_ms,
    ROUND(max_exec_time::numeric, 2) AS max_exec_ms,
    ROUND(total_exec_time::numeric, 2) AS total_exec_ms,
    rows,
    LEFT(REGEXP_REPLACE(query, '\s+', ' ', 'g'), 240) AS query
FROM pg_stat_statements
WHERE dbid = (
    SELECT oid FROM pg_database WHERE datname = current_database()
)
ORDER BY mean_exec_time DESC
LIMIT 20;
```

查看连接状态：

```sql
SELECT COALESCE(state, 'unknown') AS state, COUNT(*) AS connections
FROM pg_stat_activity
WHERE datname = current_database()
GROUP BY COALESCE(state, 'unknown')
ORDER BY state;
```

仓库中的 `scripts/benchmarks/enterprise/postgres_observability.sql` 包含完整检查 SQL，可在压测后执行并保存输出到发布记录或 PR。

## 故障排查

### API 启动失败

```bash
# 查看 API 日志
docker compose logs api

# 常见问题:
# - DATABASE_URL 连接失败 -> 检查 PostgreSQL 是否健康
# - Redis 连接失败 -> 检查 Redis 是否健康
```

### 数据库连接失败

```bash
# 检查 PostgreSQL 健康
docker compose exec postgres pg_isready -U gitai -d gitai_enterprise

# 手动连接测试
docker compose exec postgres psql -U gitai -d gitai_enterprise
```

### MinIO 连接问题

```bash
# 访问 MinIO 管理界面
# http://<server-ip>:9001
# 默认账号: minioadmin / minioadmin
```
