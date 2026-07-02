# Git-AI Report Server 部署指南

本文档面向**管理员**，介绍如何将 `git-ai report server` 部署为持久化服务，供团队开发者持续上传统计数据并通过可视化仪表盘查看分析结果。

---

## 目录

1. [快速启动（脚本方式）](#快速启动脚本方式)
2. [Docker 部署（推荐）](#docker-部署推荐)
3. [直接安装部署（无 Docker）](#直接安装部署无-docker)
4. [Linux 系统服务（systemd）](#linux-系统服务systemd)
5. [环境变量配置](#环境变量配置)
6. [数据备份与迁移](#数据备份与迁移)
7. [通知开发者上传](#通知开发者上传)

---

## 快速启动（脚本方式）

项目提供了 Linux/macOS 和 Windows 两套启动脚本，封装了常用操作。

### Linux / macOS

```bash
# 赋予执行权限（首次）
chmod +x scripts/start-server.sh

# 前台启动（调试用）
./scripts/start-server.sh

# 后台守护进程模式
./scripts/start-server.sh --daemon

# 自定义地址和数据库路径
./scripts/start-server.sh --daemon --addr 0.0.0.0:8787 --db /data/git-ai/report.sqlite

# 使用 Docker Compose 启动
./scripts/start-server.sh --docker

# 重新构建镜像后启动
./scripts/start-server.sh --docker --build

# 查看状态
./scripts/start-server.sh --status

# 停止后台进程
./scripts/start-server.sh --stop
```

### Windows (PowerShell)

```powershell
# 前台启动（调试用）
.\scripts\start-server.ps1

# 后台进程模式
.\scripts\start-server.ps1 -Daemon

# 自定义地址和数据库路径
.\scripts\start-server.ps1 -Daemon -Addr "0.0.0.0:8787" -Db "D:\data\report.sqlite"

# 使用 Docker Compose 启动
.\scripts\start-server.ps1 -Docker

# 重新构建镜像后启动
.\scripts\start-server.ps1 -Docker -Build

# 查看状态
.\scripts\start-server.ps1 -Status

# 停止后台进程
.\scripts\start-server.ps1 -Stop
```

---

## Docker 部署（推荐）

Docker 部署是生产环境的首选方案，无需安装 Rust 工具链，镜像包含完整的运行时依赖。

### 前提条件

- [Docker](https://docs.docker.com/get-docker/) 20.10+
- [Docker Compose](https://docs.docker.com/compose/install/) v2+

### 一键启动

```bash
# 1. 克隆仓库（或直接在项目目录中执行）
git clone https://github.com/git-ai-project/git-ai.git
cd git-ai

# 2. 创建数据目录
mkdir -p data

# 3. 构建并启动
docker compose up -d

# 4. 验证服务就绪
curl http://localhost:8787/api/v1/aggregate/summary
```

浏览器访问 `http://your-server-ip:8787/` 查看可视化仪表盘。

### 使用预构建镜像（不重新编译）

如果不想本地编译，可以直接用发布镜像（在有发布到 registry 的情况下）：

```bash
# 使用已发布的镜像，跳过 build
docker run -d \
  --name git-ai-report \
  --restart unless-stopped \
  -p 8787:8787 \
  -v "$(pwd)/data:/data" \
  git-ai-report:latest
```

### 配置端口和数据目录

通过 `.env` 文件自定义配置（与 `docker-compose.yml` 同目录）：

```bash
# .env
REPORT_PORT=9090          # 宿主机监听端口（默认 8787）
DATA_DIR=/mnt/disk/git-ai-data  # 数据库持久化目录（默认 ./data）
```

然后重启：

```bash
docker compose up -d --force-recreate
```

### 常用 Docker 运维命令

```bash
# 查看实时日志
docker compose logs -f

# 查看容器状态
docker compose ps

# 重启服务
docker compose restart

# 停止并删除容器（数据卷保留）
docker compose down

# 停止并删除容器和数据卷（危险！数据会丢失）
docker compose down -v

# 进入容器 shell
docker exec -it git-ai-report-server sh
```

### 健康检查

`docker-compose.yml` 内置了健康检查，每 30 秒检测一次 API 响应：

```bash
# 查看健康状态
docker inspect git-ai-report-server --format='{{.State.Health.Status}}'
# 输出: healthy / unhealthy / starting
```

---

## 直接安装部署（无 Docker）

### 安装 git-ai

```bash
# Linux / macOS
curl -sSL https://usegitai.com/install.sh | bash

# Windows
powershell -NoProfile -ExecutionPolicy Bypass -Command "irm https://usegitai.com/install.ps1 | iex"
```

### 或从源码编译

```bash
# 需要 Rust 1.93+
cargo build --release
# 二进制位于 target/release/git-ai
```

### 手动启动

```bash
# Linux / macOS
git-ai report server --addr 0.0.0.0:8787 --db /data/report.sqlite

# Windows
git-ai.exe report server --addr 0.0.0.0:8787 --db C:\data\report.sqlite
```

---

## Linux 系统服务（systemd）

适合在 Linux 服务器上长期运行，开机自动启动。

### 创建 service 文件

```bash
sudo tee /etc/systemd/system/git-ai-report.service > /dev/null <<EOF
[Unit]
Description=Git-AI Report Server
Documentation=https://usegitai.com/docs
After=network.target

[Service]
Type=simple
User=www-data
Group=www-data
ExecStart=/usr/local/bin/git-ai report server --addr 0.0.0.0:8787 --db /data/git-ai/report.sqlite
Restart=on-failure
RestartSec=5
StandardOutput=journal
StandardError=journal
SyslogIdentifier=git-ai-report

# 安全加固
NoNewPrivileges=yes
ProtectSystem=strict
ReadWritePaths=/data/git-ai

[Install]
WantedBy=multi-user.target
EOF
```

### 启动服务

```bash
# 创建数据目录并赋权
sudo mkdir -p /data/git-ai
sudo chown www-data:www-data /data/git-ai

# 启用并启动
sudo systemctl daemon-reload
sudo systemctl enable git-ai-report
sudo systemctl start git-ai-report

# 查看状态
sudo systemctl status git-ai-report

# 查看日志
sudo journalctl -u git-ai-report -f
```

---

## 环境变量配置

启动脚本和 Docker 镜像均支持通过环境变量覆盖默认值：

| 变量 | 说明 | 默认值 |
|------|------|--------|
| `GIT_AI_REPORT_ADDR` | 服务监听地址 | `0.0.0.0:8787` |
| `GIT_AI_REPORT_DB` | SQLite 数据库文件路径 | `./data/report.sqlite` |
| `GIT_AI_REPORT_LOG` | 日志文件路径（脚本后台模式） | `./data/server.log` |
| `GIT_AI_REPORT_PID` | PID 文件路径（脚本后台模式） | `./data/server.pid` |
| `REPORT_PORT` | Docker Compose 宿主机端口 | `8787` |
| `DATA_DIR` | Docker Compose 数据目录 | `./data` |

示例：

```bash
export GIT_AI_REPORT_ADDR="0.0.0.0:9090"
export GIT_AI_REPORT_DB="/mnt/nfs/git-ai/report.sqlite"
./scripts/start-server.sh --daemon
```

---

## 数据备份与迁移

所有数据存储在单个 SQLite 文件中，备份非常简单。

### 热备份（服务运行中）

```bash
# SQLite 内置的在线备份（不影响服务运行）
sqlite3 /data/git-ai/report.sqlite ".backup /backup/report-$(date +%Y%m%d).sqlite"
```

### 定时备份（cron）

```bash
# 每天凌晨 2 点备份，保留最近 30 天
crontab -e
# 添加：
0 2 * * * sqlite3 /data/git-ai/report.sqlite ".backup /backup/git-ai-report-$(date +\%Y\%m\%d).sqlite" && find /backup -name "git-ai-report-*.sqlite" -mtime +30 -delete
```

### 迁移到新服务器

```bash
# 1. 停止旧服务
./scripts/start-server.sh --stop

# 2. 复制数据库文件
scp /data/git-ai/report.sqlite user@new-server:/data/git-ai/report.sqlite

# 3. 在新服务器启动服务
./scripts/start-server.sh --daemon --db /data/git-ai/report.sqlite
```

---

## 通知开发者上传

服务器启动后，将以下信息发给团队开发者：

```
Git-AI 统计服务器已就绪！

上传命令（在你的项目根目录执行）：
  git-ai report summary --server http://your-server-ip:8787 \
    --org "你的组织名" \
    --dept "你的部门名"

仪表盘地址：http://your-server-ip:8787/

建议每次发布前或每周定期上传一次。
```

### 在 CI/CD 中自动上传示例

**GitHub Actions：**
```yaml
- name: Upload AI Usage Report
  run: |
    git-ai report summary \
      --server ${{ secrets.GIT_AI_REPORT_SERVER }} \
      --org "${{ vars.ORGANIZATION }}" \
      --dept "${{ vars.DEPARTMENT }}" \
      --period "$(date +%Y-Q$(( ($(date +%-m)-1)/3+1 )))"
```

**GitLab CI：**
```yaml
upload-ai-report:
  stage: post-deploy
  script:
    - git-ai report summary
        --server $GIT_AI_REPORT_SERVER
        --org "$CI_PROJECT_NAMESPACE"
        --dept "$DEPARTMENT"
  only:
    - main
```
