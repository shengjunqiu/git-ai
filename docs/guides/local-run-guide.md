# 本地完整运行指南

本文档说明如何在本机把 `git-ai` 项目完整跑起来。这里的“完整”包括主 CLI/daemon、本地 report dashboard、企业服务端，以及可选的编辑器/agent 插件项目。

## 1. 项目运行模型

本仓库不是单一前端项目，而是一个产品型 monorepo：

- 根目录：`git-ai` CLI、Git 代理、daemon、report server。
- `enterprise-server/`：企业服务端，包含 API、PostgreSQL、Redis、MinIO、内嵌 dashboard 页面。
- `agent-support/vscode/`：VS Code/Cursor/Windsurf 扩展。
- `agent-support/opencode/`：OpenCode 插件。
- `agent-support/intellij/`：IntelliJ 插件。

前端没有独立 Web 工程。Dashboard 页面主要以内嵌 HTML/CSS/JavaScript 的形式写在 Rust handler 里，例如 `enterprise-server/src/handlers/dashboard.rs` 和 `src/report/server.rs`。

## 2. 前置依赖

必需：

```bash
rustc --version
cargo --version
git --version
task --version
docker compose version
```

可选：

```bash
node --version
yarn --version
java --version
```

用途：

- Rust/Cargo：构建主 CLI 和 enterprise server。
- Taskfile：运行仓库约定的构建、测试、开发命令。
- Docker Compose：启动 enterprise server 依赖服务或全量容器化服务。
- Node/Yarn：运行 VS Code 和 OpenCode 插件项目。
- Java/Gradle：运行 IntelliJ 插件项目。

## 3. 主 CLI/Daemon

在仓库根目录运行：

```bash
task build
```

这只用于检查项目能否编译。

真正本地开发运行请使用：

```bash
task dev
```

`task dev` 会执行 debug 构建，安装本地 `git-ai`，运行安装流程，并重启 daemon。这个命令会影响本机 Git 调用路径，让普通 `git` 命令通过 git-ai 代理工作。

验证：

```bash
git-ai version
git-ai status
```

在任意 Git 仓库里做一次普通提交：

```bash
git status
git add .
git commit -m "Test git-ai locally"
git-ai blame path/to/file
```

调试单个命令时也可以直接用 Cargo：

```bash
cargo run -- version
cargo run -- status
```

模拟 `git` 代理模式：

```bash
GIT_AI=git cargo run -- status
```

## 4. 本地 Report Dashboard

本地 report dashboard 是轻量服务，使用 SQLite，不需要 PostgreSQL/Redis/MinIO。

启动：

```bash
git-ai report server --addr 127.0.0.1:8787 --db data/report.sqlite
```

访问：

```text
http://127.0.0.1:8787/
http://127.0.0.1:8787/dashboard
```

另开一个终端，上传当前仓库摘要：

```bash
git-ai report summary . \
  --server http://127.0.0.1:8787 \
  --organization "Local" \
  --department "Dev" \
  --reporter-name "Developer" \
  --reporter-email "developer@example.com" \
  --report-period "local"
```

也可以只扫描，不上传：

```bash
git-ai report scan .
git-ai report scan . --json
```

导出报告：

```bash
git-ai report export . --format json --output report.json
git-ai report export . --format csv --output report.csv
```

也可以用脚本启动本地 report server：

```bash
./scripts/start-server.sh
./scripts/start-server.sh --daemon
./scripts/start-server.sh --status
./scripts/start-server.sh --stop
```

默认脚本配置：

- 地址：`0.0.0.0:8787`
- 数据库：`./data/report.sqlite`
- 日志：`./data/server.log`
- PID：`./data/server.pid`

## 5. Enterprise Server

Enterprise Server 是独立服务，默认端口为 `8080`，依赖：

- PostgreSQL
- Redis
- MinIO

有两种运行方式。

### 方式 A：全 Docker Compose 运行

进入子项目：

```bash
cd enterprise-server
cp .env.example .env
```

编辑 `.env`，至少确认：

```bash
JWT_SECRET=change-me-to-a-strong-random-string-at-least-32-chars
BASE_URL=http://localhost:8080
```

启动全部服务：

```bash
docker compose up -d --build
```

验证：

```bash
curl http://localhost:8080/health
docker compose ps
docker compose logs -f api
```

访问：

```text
API:       http://localhost:8080
Dashboard: http://localhost:8080/me
Login:     http://localhost:8080/login
Health:    http://localhost:8080/health
MinIO:     http://localhost:9001
```

停止：

```bash
docker compose down
```

删除数据库和对象存储卷：

```bash
docker compose down -v
```

### 方式 B：Docker 起依赖，Cargo 跑 API

这种方式适合调试 enterprise server Rust 代码。

进入子项目：

```bash
cd enterprise-server
cp .env.example .env
```

只启动依赖服务：

```bash
docker compose up -d postgres redis minio
```

如果用 `cargo run` 从宿主机启动 API，`.env` 需要写宿主机可访问地址：

```bash
DATABASE_URL=postgresql://gitai:gitai@localhost:5433/gitai_enterprise
REDIS_URL=redis://localhost:6379
JWT_SECRET=change-me-to-a-strong-random-string-at-least-32-chars
S3_ENDPOINT=http://localhost:9000
S3_BUCKET=git-ai-cas
S3_ACCESS_KEY=minioadmin
S3_SECRET_KEY=minioadmin
S3_REGION=us-east-1
BASE_URL=http://localhost:8080
```

运行迁移。注意当前代码使用的是 `--migrate` flag：

```bash
cargo run -- --migrate
```

启动 API：

```bash
cargo run
```

验证：

```bash
curl http://localhost:8080/health
```

打开：

```text
http://localhost:8080/me
```

## 6. 客户端连接 Enterprise Server

主 `git-ai` 客户端可以指向本地 enterprise server：

```bash
git-ai config set api_base_url http://localhost:8080
git-ai login --server http://localhost:8080
```

登录后可以访问：

```text
http://localhost:8080/me
```

Dashboard 登录页支持两类凭据：

- `git-ai login --server http://localhost:8080` 获取的 Bearer token。
- 通过 admin API 创建的 `gai_...` API key。

## 7. 插件项目

这些不是完整运行主系统的必需步骤，只在开发插件时需要。

### VS Code / Cursor / Windsurf 扩展

```bash
cd agent-support/vscode
yarn install
yarn compile
yarn test
```

打包：

```bash
yarn package
```

### OpenCode 插件

```bash
cd agent-support/opencode
yarn install
yarn type-check
```

### IntelliJ 插件

```bash
cd agent-support/intellij
./gradlew build
```

运行插件沙箱：

```bash
./gradlew runIde
```

## 8. 推荐完整启动顺序

第一次本地跑，建议按这个顺序：

```bash
# 1. 根目录，确认主项目能编译
task build

# 2. 安装本地 debug 版 git-ai，并启动/重启 daemon
task dev

# 3. 验证 CLI
git-ai version
git-ai status

# 4. 启动本地 report dashboard
git-ai report server --addr 127.0.0.1:8787 --db data/report.sqlite
```

另开终端：

```bash
# 5. 给本地 dashboard 上传摘要
git-ai report summary . --server http://127.0.0.1:8787
```

确认 `http://127.0.0.1:8787/dashboard` 能看到数据后，再启动 enterprise server：

```bash
cd enterprise-server
cp .env.example .env
docker compose up -d --build
curl http://localhost:8080/health
```

## 9. 常用端口

| 端口 | 服务 |
| --- | --- |
| `8787` | 本地 `git-ai report server` |
| `8080` | Enterprise Server API/Dashboard |
| `5433` | Enterprise PostgreSQL 暴露到宿主机的端口 |
| `6379` | Redis |
| `9000` | MinIO S3 API |
| `9001` | MinIO Console |

如果端口被占用，可以改命令参数：

```bash
git-ai report server --addr 127.0.0.1:8877 --db data/report.sqlite
```

Enterprise Server 可改：

```bash
LISTEN_ADDR=0.0.0.0:8081 cargo run
```

Docker Compose 模式则需要改 `enterprise-server/docker-compose.yml` 的端口映射。

## 10. 测试与质量检查

根目录：

```bash
task test
task lint
task format
```

指定测试：

```bash
task test TEST_FILTER=report
task test TEST_FILTER=checkpoint NO_CAPTURE=true
```

只检查格式：

```bash
task format
```

快照测试更新：

```bash
cargo insta review
cargo insta accept
```

## 11. 常见问题

### `cargo run` 启动 enterprise server 报缺少 `DATABASE_URL`

`enterprise-server/.env.example` 主要为 Docker Compose 提供默认说明。用宿主机 `cargo run` 时，需要在 `.env` 里显式写：

```bash
DATABASE_URL=postgresql://gitai:gitai@localhost:5433/gitai_enterprise
REDIS_URL=redis://localhost:6379
JWT_SECRET=change-me-to-a-strong-random-string-at-least-32-chars
```

### 迁移命令应该怎么写

当前代码里是：

```bash
cargo run -- --migrate
```

不是 `cargo run -- migrate`。

### Report dashboard 没有数据

先确认服务在运行：

```bash
curl http://127.0.0.1:8787/api/v1/aggregate/summary
```

再上传摘要：

```bash
git-ai report summary . --server http://127.0.0.1:8787
```

### `task dev` 后 Git 行为变了

这是预期行为。`task dev` 会安装本地 debug 版并让 Git 走 git-ai 代理，用于真实验证 checkpoint、commit hook、daemon 和 authorship note。

### Docker 拉镜像失败

默认 Docker 配置使用官方镜像。如果你在旧版本或本地改动里看到类似 `docker.m.daocloud.io/library/postgres:16-alpine` 的镜像源地址，并遇到镜像源不可用，可以换成官方镜像：

```yaml
postgres:16-alpine
redis:7-alpine
minio/minio:latest
```

### 清理本地生成物

只预览：

```bash
git clean -ndX
```

确认无误后删除被 `.gitignore` 覆盖的生成物：

```bash
git clean -fdX
```

这会删除 `target/`、`dist/`、本地 SQLite、日志等忽略文件。不要用它删除未忽略的源码文件。
