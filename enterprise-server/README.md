# git-ai Enterprise Server

与 git-ai 客户端完全兼容的企业服务端实现。

## 快速开始

```bash
# 1. 配置环境变量
cp .env.example .env
# 编辑 .env 设置数据库连接、JWT 密钥等

# 2. 启动依赖服务 (PostgreSQL, Redis, MinIO)
docker compose up -d postgres redis minio

# 3. 运行数据库迁移
cargo run -- migrate

# 4. 启动服务
cargo run
```

## 配置

所有配置通过环境变量或 `.env` 文件加载：

| 变量 | 说明 | 默认值 |
|------|------|--------|
| `DATABASE_URL` | PostgreSQL 连接字符串 | `postgresql://gitai:gitai@localhost:5432/gitai_enterprise` |
| `REDIS_URL` | Redis 连接字符串 | `redis://localhost:6379` |
| `JWT_SECRET` | JWT 签名密钥 | (必需) |
| `S3_ENDPOINT` | MinIO/S3 端点 | `http://localhost:9000` |
| `S3_BUCKET` | CAS 存储桶名 | `git-ai-cas` |
| `S3_ACCESS_KEY` | S3 访问密钥 | `minioadmin` |
| `S3_SECRET_KEY` | S3 秘密密钥 | `minioadmin` |
| `LISTEN_ADDR` | 监听地址 | `0.0.0.0:8080` |
| `BASE_URL` | 可信公开 Origin；用于 OAuth、Dashboard、CLI 安装命令和发布下载；生产必须使用 HTTPS | `http://localhost:8080` |
| `ALLOW_INSECURE_PUBLIC_URL` | 仅开发环境显式允许非回环 HTTP 地址 | `false` |

## 客户端配置

使用浏览器 OAuth 登录时，`--server` 会自动保存服务地址并将凭据绑定到该服务器：

```bash
git-ai login --server https://your-enterprise-server.com
```

使用 API key 或自动化环境时，也可以显式配置服务地址：

```bash
# 方法 1: 环境变量
export GIT_AI_API_BASE_URL=https://your-enterprise-server.com
export GIT_AI_API_KEY=your-api-key

# 方法 2: 配置文件
git-ai config set api_base_url https://your-enterprise-server.com
git-ai config set api_key your-api-key
```

## API 端点

### 认证
- `POST /worker/oauth/device/code` — 启动设备授权
- `POST /worker/oauth/token` — 令牌交换 (3 种 grant_type)

### 数据上传
- `POST /worker/metrics/upload` — 批量上传 Metrics 事件
- `POST /worker/cas/upload` — 批量上传 CAS 对象
- `GET /worker/cas/?hashes=` — 批量读取 CAS 对象

### 报告
- `POST /api/v1/reports` — 上传报告
- `POST /api/v1/summaries` — 上传摘要
- `POST /api/bundles` — 创建分享 Bundle

### 版本分发
- `GET /worker/releases` — 获取版本信息
- `GET /worker/releases/{channel}/download/{filename}` — 下载文件

### 看板
- `GET /me` — Dashboard 首页
- `GET /api/v1/aggregate/summary` — 全局汇总
- `GET /api/v1/aggregate/organizations` — 按组织聚合
- `GET /api/v1/aggregate/departments` — 按部门聚合
- `GET /api/v1/aggregate/projects` — 按项目聚合
- `GET /api/v1/aggregate/developers` — 按开发者聚合

### 管理
- `GET /worker/config/feature-flags` — Feature Flags
