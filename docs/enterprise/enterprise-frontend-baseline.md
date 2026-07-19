# Enterprise 前端阶段 0 基线

记录日期：2026-07-19

基线分支：`codex/enterprise-frontend-phase-0`

对应计划：[`enterprise-frontend-optimization-task-plan.md`](./enterprise-frontend-optimization-task-plan.md)

## 1. 范围与环境

阶段 0 只记录现状，不修改 Dashboard、handler、API 或部署配置。

允许修改的文件：

- 本文档。
- `docs/enterprise/baselines/frontend-phase-0/` 下的基线截图。
- 总体任务清单中阶段 0 的执行状态。

开始执行时：

- `git status --short --branch` 显示 `main...origin/main`，工作区干净。
- 已确认独立的自动刷新专项文档
  [`dashboard-auto-refresh-flicker-optimization-task-plan.md`](./dashboard-auto-refresh-flicker-optimization-task-plan.md)
  存在，本阶段没有覆盖或修改它。
- 已创建独立分支 `codex/enterprise-frontend-phase-0`。

本地浏览器基线使用仓库现有 Docker Compose 环境：

- API：`http://127.0.0.1:8080`
- PostgreSQL、Redis、MinIO 和 API 容器在采集结束后均恢复为健康状态。
- 开发者视图使用现有基准测试 member 账号。
- 管理员视图使用本轮临时创建的 owner 账号；采集完成后已删除该账号及其 4 条关联审计记录。
- 没有执行创建用户、删除业务用户、授权、撤销、上传、发布或设置类 UI mutation。

## 2. 文件与资源规模

| 文件 | 行数 | 原始字节 | gzip 字节 |
| --- | ---: | ---: | ---: |
| `enterprise-server/static/dashboard.html` | 672 | 48,663 | 8,744 |
| `enterprise-server/static/dashboard.css` | 345 | 27,039 | 5,479 |
| `enterprise-server/static/dashboard.js` | 1,954 | 89,477 | 18,039 |
| 合计 | 2,971 | 165,179 | 32,262 |

首屏依赖：

- 同源 `dashboard.css`。
- 同源 `dashboard.js`。
- 外部 `https://cdn.jsdelivr.net/npm/chart.js@4.4.7/dist/chart.umd.min.js`。
- HTML 中的内联角色变量和内联脚本。

静态资源响应使用 `Cache-Control: no-cache`。Dashboard HTML 和 API 响应没有
`Content-Security-Policy`、`X-Content-Type-Options`、`Referrer-Policy` 或
`X-Frame-Options` 响应头；三类响应均带 `Access-Control-Allow-Origin: *`。

Web session Cookie 的代码基线为 `HttpOnly; SameSite=Lax`，只有配置的基础地址以
`https://` 开头时才增加 `Secure`。

## 3. 现有检查

| 命令 | 结果 | 备注 |
| --- | --- | --- |
| `node --check enterprise-server/static/dashboard.js` | 通过 | 无输出，退出码 0 |
| `cargo test --manifest-path enterprise-server/Cargo.toml` | 通过 | 163 passed，0 failed；编译期间已有 62 条 warning |
| `task lint` | 失败 | 与 Enterprise 前端无关的既有 MDM agent lint 错误 |

`task lint` 的既有失败：

- `src/mdm/agents/codebuddy.rs`：未使用 import；另有一个 `needless_return`。
- `src/mdm/agents/qoder.rs`：未使用 import。
- `src/mdm/agents/trae.rs`：未使用 import。

本阶段不修复这些根仓库既有问题。

## 4. Dashboard 栏目与角色

| 栏目 key | 页面名称 | 管理员 | 普通开发者 | 主要读取 |
| --- | --- | --- | --- | --- |
| `overview` | 总览 | 可见 | 可见 | 汇总、Top 开发者、趋势；开发者另读 CLI 状态 |
| `trends` | 趋势分析 | 可见 | 可见 | 趋势和 agent 对比 |
| `organizations` | 组织 | 可见 | 隐藏 | 组织聚合 |
| `departments` | 部门 | 全组织树 | 仅所属部门范围 | 部门聚合 |
| `developers` | 开发者 | 可见 | 可见 | 开发者聚合和 Git identity |
| `projects` | 项目 | 可见 | 可见 | 项目聚合 |
| `tools` | AI 工具 | 可见 | 可见 | 工具/模型聚合 |
| `users` | 用户管理 | 可见 | 隐藏 | 管理员用户列表 |
| `apikeys` | API 密钥 | 可见 | 隐藏 | 管理员密钥列表 |
| `releases` | CLI 版本发布 | 可见 | 隐藏 | release channel 和资产 |
| `files` | 文件中心 | 可见 | 隐藏 | 托管文件和版本 |
| `help` | 安装与使用指南 | 可见 | 可见 | 静态帮助内容 |

前端会隐藏管理员栏目并拒绝把普通开发者路由到管理员 section；这只是展示限制，
服务端仍必须是权限边界。实测未认证访问管理 API 返回 401，member 访问管理 API 返回
403，member 读取普通聚合 API 返回 200。

## 5. 管理 mutation 清单

| 功能 | 方法与端点 | 当前前端保护 |
| --- | --- | --- |
| 批量授权 Git 追踪上传 | `POST /api/admin/users/git-tracking-upload/authorize` | `confirm()`；操作期间禁用按钮 |
| 单用户授权或撤销 Git 追踪上传 | `PUT /api/admin/users/{id}/git-tracking-upload` | `confirm()`；操作期间禁用按钮 |
| 创建用户 | `POST /api/admin/users` | 表单校验；没有统一请求层 |
| 删除用户 | `DELETE /api/admin/users/{id}` | `confirm()`，说明不可撤销 |
| 创建部门 | `POST /api/admin/departments` | 模态表单校验 |
| 创建当前管理员 API Key | `POST /api/admin/api-keys` | 表单校验 |
| 为指定用户创建 API Key | `POST /api/admin/api-keys` | 表单校验 |
| 撤销 API Key | `DELETE /api/admin/api-keys/{id}` | `confirm()` |
| 上传并发布 CLI 版本 | `POST /api/admin/releases/publish` | 校验版本号和 6 个平台文件；操作期间禁用按钮 |
| 切换 `latest` | `POST /api/admin/releases/channel` | `confirm()` |
| 上传普通文件 | `POST /api/admin/files/upload` | 必填校验；操作期间禁用按钮 |
| 上传后立即发布普通文件 | `POST /api/admin/files/{slug}/publish` | 与上传串行执行 |
| 发布已有文件版本 | `POST /api/admin/files/{slug}/publish` | `confirm()` |
| 删除文件版本 | `DELETE /api/admin/files/{slug}/versions/{version}` | `confirm()`，说明不可撤销 |
| 修改文件名称、说明和公开状态 | `PUT /api/admin/files/{slug}` | 表单校验 |

所有 mutation 当前直接调用 `fetch()`，没有自动重试。创建用户、创建部门和创建 API Key
没有完整的一致性防重复提交机制。用户列表每次执行 `loadUsers()` 都会清空
`selectedGitTrackingUserIds`，所以自动刷新会丢失批量选择。

## 6. 认证与服务端页面

| 路径 | handler | 用途 |
| --- | --- | --- |
| `/auth/login` | `auth_pages.rs` + `auth_api.rs` | 邮箱密码登录 |
| `/auth/register` | `auth_pages.rs` + `auth_api.rs` | 组织/部门注册 |
| 注册成功页 | `auth_pages.rs::success_page` | 注册结果和登录入口 |
| `/login` | `login.rs` | 旧版 token/API key 登录 |
| `/logout`、`/auth/logout` | `login.rs`、`auth_api.rs` | 清理认证 Cookie |
| `/auth/cli/authorize` | `cli_authorize.rs` | CLI 浏览器授权 |
| `/verify` | `verify.rs` | OAuth 设备码验证 |
| `/bundle/{id}` | `bundle_view.rs` | 公共 bundle 展示页，不属于登录流程但同样直接拼接页面 |

这些页面分别内嵌 HTML、CSS 或 JavaScript，目前没有共享页面布局或统一的 `auth.css`。

## 7. 外部资源与硬编码部署地址

生产页面依赖一个外部资源：

- Chart.js 4.4.7，来自 jsDelivr CDN；没有本地 fallback。

帮助页包含 20 处固定的 `117.147.213.234:38080` 地址，覆盖：

- 注册页和旧登录页链接。
- VSIX 下载链接。
- CLI `--server` 参数。
- macOS/Linux `curl ... | bash`。
- Windows `irm ... | iex`。
- 健康检查、网络诊断和预期配置说明。

所有固定地址均为 HTTP。浏览器实测帮助页会直接展示这些固定地址和管道执行命令。

## 8. 浏览器、网络与刷新基线

### 初次加载

开发者总览首轮加载：

- 1 个 HTML navigation。
- 3 个页面资源：Chart.js、CSS、JavaScript。
- 4 个 API 请求：summary、Top developers、trends、client status。
- 合计 8 个请求。

管理员总览首轮加载不请求 client status，合计 7 个请求。

本地页面资源清单在等待 60 秒后观察到第二组 summary、developers、trends 请求，并观察到
client status 的第二次 fetch 来源。开发者总览每轮自动刷新重新发起 4 个 API 请求；
当前路径会重新执行 loader，没有静默 diff 或 Chart 实例复用保证。

### 部门页

本地库有 418 个部门。初次进入部门页通过 `limit=100` 串行请求 5 页
`/api/v1/aggregate/departments`，连同 HTML 和 3 个页面资源共 9 个请求。

打开“新增部门”模态框还会请求：

- 1 页管理员组织列表。
- 5 页管理员部门列表，用于父部门下拉框。

代码上限为 50 页、每页 100 条，即单次全量读取最多 5,000 条。

### 慢速、离线和错误

| 场景 | 结果 |
| --- | --- |
| 16 KiB/s 限速下载 `dashboard.js` | 89,477 字节，4.512 秒，HTTP 200 |
| API 容器停止 | 连接失败，curl 状态 000 |
| 未认证管理 API | HTTP 401 |
| member 访问管理 API | HTTP 403 |
| PostgreSQL 停止时读取 summary | HTTP 500 |

现有脚本没有统一的 401/403 跳转或会话恢复逻辑；各 loader 的错误展示方式不一致。自动刷新
失败时部分栏目会用错误行覆盖旧内容。

受当前自动化浏览器能力限制，本轮没有取得 DevTools Performance 中的 Layout/Paint 明细，
也没有浏览器级 Network throttling waterfall。阶段 4 前应补一份人工 DevTools trace，
作为本阶段唯一未自动化的性能基线项。

当前浏览器只暴露 viewport、页面资源清单和截图能力，没有 performance trace 或 CDP
metrics 能力；页面执行上下文也不允许读取 `window.performance`。人工补采步骤：

1. 使用开发者账号打开总览，在 DevTools Performance 中开启 Screenshots。
2. 录制一次重新加载，待总览三个 API 请求和 client status 完成后停止，导出为
   `overview-initial.json.gz`。
3. 保持总览不操作，开始第二次录制并等待一轮 60 秒自动刷新完成，导出为
   `overview-auto-refresh.json.gz`。
4. 在本文档补充两次录制的 Layout、Paint、长任务、图表动画和截图时间线结论。

## 9. 响应式、键盘与模态框现状

- `dashboard.css` 在 `max-width: 768px` 直接执行 `.sidebar { display: none; }`。
- 390 × 844 下没有菜单按钮、底部导航或其他 section 切换入口，也没有可见退出入口。
- 用户管理表格在 390px 下发生明显横向溢出，行高被长邮箱和操作列显著撑大。
- 发布管理和帮助页在移动端只能通过直接 URL 到达，页面内无法切换到其他栏目。
- “新增部门”模态框没有 `role="dialog"` 或 `aria-modal="true"`。
- 打开该模态框后按一次 Tab，焦点落回底层“+ 新增部门”按钮，说明没有焦点圈定。
- 当前模态框没有 Escape 关闭处理，打开和关闭时也没有可靠的焦点恢复协议。

桌面页面的主要 Tab 顺序从侧边栏导航、退出按钮进入当前栏目的筛选和操作按钮；大量动态
按钮依赖 inline handler。

## 10. 截图索引

所有截图都使用本地测试数据，按计划记录 1440 × 900、1024 × 768 和 390 × 844。

| 页面 | Desktop | Tablet | Mobile |
| --- | --- | --- | --- |
| 管理员总览 | [PNG](./baselines/frontend-phase-0/admin-overview-desktop.png) | [PNG](./baselines/frontend-phase-0/admin-overview-tablet.png) | [PNG](./baselines/frontend-phase-0/admin-overview-mobile.png) |
| 管理员部门 | [PNG](./baselines/frontend-phase-0/admin-departments-desktop.png) | [PNG](./baselines/frontend-phase-0/admin-departments-tablet.png) | [PNG](./baselines/frontend-phase-0/admin-departments-mobile.png) |
| 管理员用户管理 | [PNG](./baselines/frontend-phase-0/admin-users-desktop.png) | [PNG](./baselines/frontend-phase-0/admin-users-tablet.png) | [PNG](./baselines/frontend-phase-0/admin-users-mobile.png) |
| 管理员发布管理 | [PNG](./baselines/frontend-phase-0/admin-releases-desktop.png) | [PNG](./baselines/frontend-phase-0/admin-releases-tablet.png) | [PNG](./baselines/frontend-phase-0/admin-releases-mobile.png) |
| 开发者总览 | [PNG](./baselines/frontend-phase-0/developer-overview-desktop.png) | [PNG](./baselines/frontend-phase-0/developer-overview-tablet.png) | [PNG](./baselines/frontend-phase-0/developer-overview-mobile.png) |
| 开发者帮助 | [PNG](./baselines/frontend-phase-0/developer-help-desktop.png) | [PNG](./baselines/frontend-phase-0/developer-help-tablet.png) | [PNG](./baselines/frontend-phase-0/developer-help-mobile.png) |

## 11. 阶段 0 结论

已建立可重复的代码、测试、角色、请求、错误响应和三视口截图基线。进入阶段 1 时应以以下
变化作为第一组量化对比：

1. 固定 HTTP 地址数量从 20 降为 0。
2. 生产页面外部关键 CDN 依赖从 1 降为 0。
3. 帮助页不再推荐 HTTP 管道执行安装脚本。
4. 增加明确的安全响应头基线。
5. API Key 展示不再经过 `innerHTML`。

已知剩余基线工作只有人工 DevTools Performance trace；不阻塞安全与部署可移植性阶段的
代码实施，但应在阶段 4 自动刷新优化前补齐。
