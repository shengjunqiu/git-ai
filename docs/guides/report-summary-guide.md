# Git-AI 编码分析上报 & 可视化分析使用指南

本文档面向两类角色：

- **开发者** — 如何查看本地 AI 编码统计，以及如何将项目摘要上传到统计服务器
- **管理员** — 如何启动统计服务器、查看跨团队可视化分析仪表盘

---

## 目录

1. [开发者：查看本地统计](#开发者查看本地统计)
2. [开发者：上传摘要到服务器](#开发者上传摘要到服务器)
3. [管理员：启动报表服务器](#管理员启动报表服务器)
4. [管理员：可视化分析仪表盘](#管理员可视化分析仪表盘)
5. [完整命令参考](#完整命令参考)
6. [API 端点参考](#api-端点参考)
7. [注意事项](#注意事项)

---

## 开发者：查看本地统计

### 查看当前仓库 AI 归因统计

```bash
# 在仓库目录内运行，输出各提交 AI/人工占比
git-ai report summary
```

输出示例（JSON）：

```json
{
  "project_name": "my-project",
  "git_url": "https://github.com/org/my-project.git",
  "branch": "main",
  "total_commits": 120,
  "developers": [
    {
      "name": "Alice",
      "email": "alice@example.com",
      "commits": 80,
      "added_lines": 5000,
      "ai_additions": 2000,
      "human_additions": 3000,
      "ai_ratio": 0.4,
      "human_ratio": 0.6
    }
  ],
  "project_ratios": {
    "ai": 0.4,
    "human": 0.6
  }
}
```

### 保存到文件

```bash
# 保存为 JSON
git-ai report summary --output summary.json

# 保存为 CSV（每行对应一位开发者）
git-ai report summary --format csv --output summary.csv
```

CSV 列说明：

| 列名 | 说明 |
|------|------|
| `project_name` | 项目名称 |
| `git_url` | Git 远程地址 |
| `branch` | 当前分支 |
| `developer` | 开发者姓名 |
| `developer_email` | 开发者邮箱 |
| `commits` | 该开发者提交次数 |
| `added_lines` | 新增行数 |
| `ai_additions` | AI 编写行数 |
| `human_additions` | 人工编写行数 |
| `ai_ratio` | 该开发者 AI 代码占比 |
| `human_ratio` | 该开发者人工代码占比 |
| `project_ai_ratio` | 项目整体 AI 占比 |
| `project_human_ratio` | 项目整体人工占比 |

### 查看指定仓库

```bash
git-ai report summary /path/to/repo --output summary.json
```

### 忽略特定文件

```bash
# 忽略 vendor 目录和 Markdown 文件
git-ai report summary --ignore "vendor/**" "*.md"
```

### AI Blame：逐行查看归因

```bash
# 查看指定文件的 AI/人工逐行归因（类似 git blame）
git-ai blame src/main.rs
```

---

## 开发者：上传摘要到服务器

向管理员确认服务器地址后（如 `http://report.company.com:8787`），使用以下命令上传。

### 直接上传

```bash
git-ai report summary --server http://report.company.com:8787
```

成功后输出：

```json
{
  "uploaded": true,
  "message": "Uploaded summary for 'my-project' to http://report.company.com:8787/api/v1/summaries.",
  "commit_count": 120
}
```

### 模拟上传（不实际发送，仅验证）

```bash
git-ai report summary --server http://report.company.com:8787 --dry-run
```

输出示例：

```json
{
  "uploaded": false,
  "message": "Dry run: would upload summary for 'my-project' (3 developers) to http://report.company.com:8787.",
  "commit_count": 120
}
```

### 附加上报元数据（组织/部门）

如果管理员要求按组织和部门聚合，可在上传时附加元数据：

```bash
git-ai report summary \
  --server http://report.company.com:8787 \
  --organization "TechDivision" \
  --department "Backend" \
  --reporter-name "Alice" \
  --reporter-email "alice@company.com" \
  --report-period "2026-Q2"
```

> **说明**：`--organization`、`--department` 等参数由管理员规定，确保与其他项目保持统一拼写，避免聚合时数据分散。

### 重复上传说明

同一项目（按 `project_name + git_url + branch` 唯一标识）重复上传会执行 **upsert**（更新已有记录），不会产生重复数据。建议在 CI/CD 流水线中配置定期上传。

### 在 CI/CD 中自动上传

```yaml
# GitHub Actions 示例
- name: Upload AI Report
  run: |
    git-ai report summary \
      --server ${{ secrets.REPORT_SERVER_URL }} \
      --organization "${{ vars.ORG_NAME }}" \
      --department "${{ vars.DEPT_NAME }}"
```

---

## 管理员：启动报表服务器

### 安装前提

- 已安装 `git-ai`（版本 1.3.x+）
- 服务器磁盘有写权限（用于 SQLite 数据库）
- 对应端口（默认 8787）已开放防火墙

### 启动服务器

```bash
# 默认：监听本机 127.0.0.1:8787，数据库文件 report.sqlite
git-ai report server

# 监听所有网卡（允许局域网/外网访问）
git-ai report server --addr 0.0.0.0:8787 --db /data/git-ai-report.sqlite

# 自定义端口
git-ai report server --addr 0.0.0.0:9090 --db /data/git-ai-report.sqlite
```

| 参数 | 说明 | 默认值 |
|------|------|--------|
| `--addr <host:port>` | 监听地址 | `127.0.0.1:8787` |
| `--db <path>` | SQLite 数据库路径（不存在会自动创建） | `report.sqlite` |

### 作为系统服务运行（Linux systemd）

```ini
# /etc/systemd/system/git-ai-report.service
[Unit]
Description=Git-AI Report Server
After=network.target

[Service]
ExecStart=/usr/local/bin/git-ai report server --addr 0.0.0.0:8787 --db /data/git-ai-report.sqlite
Restart=on-failure
User=www-data

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl enable git-ai-report
sudo systemctl start git-ai-report
sudo systemctl status git-ai-report
```

### 作为后台进程运行（Windows PowerShell）

```powershell
Start-Process -NoNewWindow `
  -FilePath "git-ai.exe" `
  -ArgumentList "report","server","--addr","0.0.0.0:8787","--db","D:\data\report.sqlite" `
  -RedirectStandardError "D:\logs\git-ai-report.log"
```

### 验证服务器运行状态

```bash
curl http://127.0.0.1:8787/api/v1/aggregate/summary
# 或在浏览器访问 http://127.0.0.1:8787/
```

---

## 管理员：可视化分析仪表盘

服务器启动后，直接在浏览器访问根路径即可打开内置可视化仪表盘，**无需额外部署前端**。

```
http://<server-addr>:8787/
# 或
http://<server-addr>:8787/dashboard
```

### 仪表盘功能说明

#### 全局概览卡片

页面顶部展示五个关键指标：

| 指标 | 说明 |
|------|------|
| 全局 AI 编码率 | 所有项目加权平均后的 AI 代码占比 |
| 全局人工编码率 | 所有项目加权平均后的人工代码占比 |
| 项目数 | 服务器已收录的不重复项目数量 |
| 组织/部门 | 已注册的组织和部门数量 |
| 开发者数 | 活跃开发者总数 |

#### 五个分析维度（Tab 切换）

| Tab | 内容 |
|-----|------|
| **总览** | 全局 AI/人工占比饼图、各组织 AI 编码率柱状图、项目 Top10、开发者 Top10 |
| **组织** | 各组织柱状图 + 详情表格，点击行可下钻到该组织的部门详情 |
| **部门** | 支持按组织过滤，展示各部门 AI 编码率对比，带面包屑导航 |
| **项目** | 支持组织/部门联动过滤，展示项目维度统计 |
| **开发者** | 支持组织/部门过滤，跨项目汇总每位开发者的 AI/人工代码行数对比 |

#### 下钻交互

- 点击**总览 → 组织柱状图**的某个柱子，自动跳转到该组织的部门详情
- 点击**组织 Tab** 的某一行，自动切换到部门 Tab 并过滤该组织
- **项目**和**开发者** Tab 支持组织/部门联动下拉过滤器

#### 数据刷新

点击右上角 **↻ 刷新** 按钮可重新拉取所有 API 数据。数据实时从服务端 API 读取，无需手动重启服务。

---

## 完整命令参考

```
git-ai report - generate AI/human project usage reports

Commands:
  scan      Scan commits and show AI/human attribution stats
  export    Export full report to JSON or CSV file
  summary   Generate simplified project summary (all history, per-developer AI ratio)
  upload    Upload report to a server
  server    Start a report ingestion server

Options:
  --range <from>..<to>      Commit range to scan
  --branch <branch>         Branch to scan
  --since <time>            Filter commits after time
  --until <time>            Filter commits before time
  --ignore <patterns>       Ignore file patterns (space-separated)
  --json                    Print JSON report
  --format <json|csv>       Export format (default: json)
  --output, -o <path>       Export output path
  --server <url>            Upload server URL
  --dry-run                 Validate upload payload without sending it
  --organization <name>     Organization name for the report
  --department <name>       Department name for the report
  --reporter-name <name>    Reporter name
  --reporter-email <email>  Reporter email
  --report-period <period>  Reporting period (e.g. 2026-Q2)
  --addr <host:port>        Report server listen address (default: 127.0.0.1:8787)
  --db <path>               Report server SQLite database path (default: report.sqlite)
```

---

## API 端点参考

以下端点均由 `git-ai report server` 提供：

### 可视化仪表盘

| 方法 | 路径 | 说明 |
|------|------|------|
| `GET` | `/` | 打开可视化分析仪表盘 |
| `GET` | `/dashboard` | 同上（别名） |

### 摘要上传与查询

| 方法 | 路径 | 说明 |
|------|------|------|
| `POST` | `/api/v1/summaries` | 接收并存储项目摘要（由 `git-ai report summary --server` 调用） |
| `GET` | `/api/v1/summaries` | 获取所有项目摘要列表 |
| `GET` | `/api/v1/summaries/:id` | 获取指定摘要详情（含开发者列表） |

### 聚合分析（仪表盘数据源）

| 方法 | 路径 | 查询参数 | 说明 |
|------|------|----------|------|
| `GET` | `/api/v1/aggregate/summary` | — | 全局统计汇总 |
| `GET` | `/api/v1/aggregate/organizations` | — | 按组织聚合统计 |
| `GET` | `/api/v1/aggregate/departments` | `?org=<name>` | 按部门聚合统计，可按组织过滤 |
| `GET` | `/api/v1/aggregate/projects` | `?org=<name>&dept=<name>` | 按项目聚合统计，可按组织/部门过滤 |
| `GET` | `/api/v1/aggregate/developers` | `?org=<name>&dept=<name>` | 按开发者聚合统计，可按组织/部门过滤 |

#### 聚合响应示例

`GET /api/v1/aggregate/summary`

```json
{
  "total_reports": 16,
  "total_projects": 16,
  "total_developers": 27,
  "total_organizations": 4,
  "total_departments": 6,
  "weighted_ai_ratio": 0.75,
  "weighted_human_ratio": 0.25
}
```

---

## 注意事项

1. **扫描范围**：`summary` 命令扫描仓库从初始提交到 HEAD 的**全部历史**，不支持 `--range`、`--since`、`--until` 过滤
2. **Schema 版本**：摘要数据使用 `git-ai-summary/1.0.0` 协议版本，服务端会校验版本一致性
3. **隐私保护**：上传时自动移除本地工作目录（`workdir`）信息；远程 URL 仅存储哈希值；不含源代码、Prompt 原文或 Transcript 内容
4. **Upsert 行为**：同一项目重复上传时，服务端更新已有记录而非创建新记录
5. **开发者归属**：统计基于 git-ai 的 authorship notes；未被 git-ai 追踪的提交会被归类为"人工"代码
6. **服务器单线程**：当前实现为单线程 TCP 服务，适合内部团队使用；生产环境建议在其前面挂 nginx 反向代理
7. **数据持久化**：所有数据存储在 SQLite 文件中，重启服务器后数据不会丢失；建议定期备份数据库文件
