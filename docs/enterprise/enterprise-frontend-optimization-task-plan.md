# Enterprise 前端总体优化任务清单

本文档把 Enterprise Server 当前前端的架构、功能、性能、安全、可访问性和工程质量问题拆成可以逐步执行、逐步验证、逐步提交的工程任务。

当前前端主要由两部分组成：

1. `enterprise-server/static/dashboard.html`、`dashboard.css`、`dashboard.js` 组成的 Dashboard。
2. `enterprise-server/src/handlers/` 中直接拼接 HTML/CSS/JS 的登录、注册、CLI 授权、设备验证和其他服务端渲染页面。

Dashboard 当前约有 12 个栏目，核心脚本接近 2,000 行，所有状态、请求、分页、图表、模态框和管理操作都位于全局作用域。这个结构适合早期无构建部署，但已经开始产生移动端不可用、请求处理不一致、自动刷新闪烁、部门全量加载、页面模板重复和缺少前端测试等问题。

本计划采用渐进式改造：先处理安全、可靠性和移动端，再处理自动刷新和大数据性能，最后拆分模块、统一模板和完善测试。不要求一开始迁移到 React、Vue 或其他大型框架。

## 执行原则

1. 每次只执行一个阶段；每个阶段验证通过后单独提交。
2. P0 功能和安全问题优先于代码风格重构。
3. 不用“延长刷新间隔”“隐藏错误”或“删除功能”代替根因修复。
4. 前端权限隐藏只能用于展示；所有管理 API 必须继续由服务端鉴权。
5. 所有请求必须经过统一请求层，不再新增裸 `fetch()`。
6. 所有服务端返回字符串默认按不可信数据处理。
7. 新模块优先使用浏览器原生 ES Modules；在现有复杂度下不强制引入框架。
8. 管理类 mutation 不自动重试，必须防止重复提交并提供明确结果。
9. 每个阶段都要覆盖管理员和普通开发者两类角色。
10. 每个阶段都记录修改文件、测试命令、结果和已知剩余问题。
11. 如果阶段实施中改变了既有 API 或部署配置，必须同步更新文档和兼容性说明。
12. 不覆盖工作区中与本任务无关的已有改动。

## 当前基线

### Dashboard 静态资源

| 文件 | 当前职责 | 当前规模 |
| --- | --- | --- |
| `enterprise-server/static/dashboard.html` | 所有栏目 DOM、帮助内容、内联变量和事件属性 | 约 672 行 |
| `enterprise-server/static/dashboard.css` | Dashboard、管理页面、帮助页面、响应式样式 | 约 344 行 |
| `enterprise-server/static/dashboard.js` | 路由、状态、API、分页、图表、模态框和管理操作 | 约 1,951 行 |

### Rust 内嵌页面

至少以下 handler 包含 HTML、CSS 或 JavaScript：

- `enterprise-server/src/handlers/auth_pages.rs`
- `enterprise-server/src/handlers/login.rs`
- `enterprise-server/src/handlers/cli_authorize.rs`
- `enterprise-server/src/handlers/verify.rs`
- `enterprise-server/src/handlers/bundle_view.rs`

### 已确认的主要问题

| 优先级 | 问题 | 影响 |
| --- | --- | --- |
| P0 | 768px 以下直接隐藏侧边栏且无替代导航 | 移动端无法正常切换栏目或退出 |
| P0 | 请求处理不一致，缺少统一 401/403 处理 | 登录过期后页面显示空数据或错误，不能可靠恢复 |
| P0 | 帮助页硬编码 HTTP IP，并提供下载后直接执行脚本的命令 | 部署不可移植，存在传输链路风险 |
| P0 | 自动刷新复用首次加载路径 | 页面闪烁、请求重叠、旧响应覆盖和状态丢失 |
| P0 | 用户列表刷新时清除批量选择 | 管理员操作可能被 60 秒刷新打断 |
| P1 | 部门页最多串行拉取 50 页、5,000 条记录 | 组织规模增大后加载和轮询成本持续增长 |
| P1 | 近 2,000 行全局脚本和大量 inline handler | 隐式耦合、难测试、难启用严格 CSP |
| P1 | Chart.js 依赖外部 CDN，未本地兜底 | 隔离网络或 CDN 故障时图表不可用 |
| P1 | 动态内容大量使用 `innerHTML` | 维护者必须手动选择正确转义上下文 |
| P1 | 静态资源统一 `Cache-Control: no-cache` | 页面导航和重新进入需要反复验证资源 |
| P2 | 模态框、Toast 和图表缺少完整可访问性 | 键盘和读屏用户体验不完整 |
| P2 | 没有 Dashboard 行为测试或浏览器端回归测试 | 结构调整和管理功能容易回归 |

## 目标架构

第一阶段目标不是引入框架，而是把当前单体脚本演进为清晰的模块边界：

```text
enterprise-server/static/
  assets/
    chart.umd.min.js
  dashboard/
    app.js
    api.js
    router.js
    state.js
    refresh.js
    pagination.js
    render.js
    ui/
      modal.js
      toast.js
    sections/
      overview.js
      trends.js
      organizations.js
      departments.js
      developers.js
      projects.js
      tools.js
      users.js
      api-keys.js
      releases.js
      files.js
  dashboard.css
  dashboard.html
  auth.css
```

栏目模块统一接口：

```js
export async function load({
    mode,
    signal,
    requestId,
}) {
    // 获取数据并提交当前栏目 UI。
}

export function dispose() {
    // 可选：释放栏目事件、图表或请求。
}
```

统一刷新模式：

```js
const RefreshMode = Object.freeze({
    INITIAL: 'initial',
    MANUAL: 'manual',
    AUTO: 'auto',
});
```

统一请求入口：

```js
await apiRequest('/api/v1/aggregate/summary', {
    method: 'GET',
    signal,
});
```

## 非目标

- 不在第一阶段直接重写为 React、Vue 或其他 SPA 框架。
- 不改变当前核心业务指标定义。
- 不用 WebSocket/SSE 替代 60 秒轮询，除非后续有独立需求和容量评估。
- 不在前端绕过或复制服务端权限判断。
- 不为了动画效果引入大型 UI 组件库。
- 不一次性重写所有 Rust handler。
- 不在没有回归验证的情况下同时修改 Dashboard、认证流程和 API 协议。
- 不把本计划中的结构优化与无关后端性能重构混在同一提交中。

## 阶段依赖与建议顺序

```text
阶段 0  基线
  |
阶段 1  安全与部署可移植性
  |
阶段 2  统一请求层
  |
阶段 3  移动端和响应式
  |
阶段 4  自动刷新专项
  |
阶段 5  大数据与静态资源性能
  |
阶段 6  事件委托和模块拆分
  |
阶段 7  服务端页面模板统一
  |
阶段 8  功能体验和可访问性
  |
阶段 9  自动化测试
  |
阶段 10 全量验收与发布
```

阶段 1～3 可以在修改文件不冲突时分支并行开发，但合并和验收仍按上述顺序执行。

## 阶段 0：建立可重复基线

目标：在修改前记录当前结构、功能、性能和安全响应，确保后续可以量化收益。

执行记录：[`enterprise-frontend-baseline.md`](./enterprise-frontend-baseline.md)

### 0.1 确认工作区状态

步骤：

- [x] 查看工作区：

```bash
git status --short
```

- [x] 记录已有未提交文件。
- [x] 确认不会覆盖现有自动刷新专项文档。
- [x] 为本次执行建立单独分支。

验收标准：

- [x] 已记录基线工作区状态。
- [x] 已明确本阶段允许修改的文件。

### 0.2 运行现有检查

步骤：

- [x] 检查 Dashboard JavaScript：

```bash
node --check enterprise-server/static/dashboard.js
```

- [x] 运行 Enterprise Server 测试：

```bash
cargo test --manifest-path enterprise-server/Cargo.toml
```

- [x] 运行根仓库 lint：

```bash
task lint
```

验收标准：

- [x] 记录每条命令的结果。
- [x] 既有失败已记录并确认与前端优化无关。

### 0.3 建立浏览器基线

步骤：

- [x] 使用管理员账号登录 Dashboard。
- [x] 使用普通开发者账号登录 Dashboard。
- [x] 记录 Desktop、Tablet、Mobile 三个 viewport：

```text
1440 × 900
1024 × 768
390 × 844
```

- [x] 对总览、部门、用户管理、发布管理和帮助页截图。
- [x] 使用 Network 面板记录初次加载资源大小和请求数（本轮由自动化浏览器资源清单与 curl 等价采集）。
- [ ] 使用 Performance 面板记录一次总览加载和一次 60 秒自动刷新（请求基线已记录；Layout/Paint trace 待人工补齐）。
- [x] 模拟慢速网络、离线、401、403 和 500。
- [x] 记录键盘 Tab 顺序和模态框行为。

验收标准：

- [x] 已保存优化前截图或录屏。
- [x] 已记录移动端导航不可用的现状。
- [x] 已记录初次加载、自动刷新和部门数据请求基线。

### 0.4 建立功能清单

- [x] 列出所有 12 个 Dashboard 栏目及角色权限。
- [x] 列出所有管理 mutation：创建、删除、授权、撤销、上传、发布和设置。
- [x] 列出所有认证相关页面。
- [x] 列出页面依赖的外部资源和硬编码部署地址。

建议提交信息：

```text
Document enterprise frontend baseline
```

## 阶段 1：修复安全与部署可移植性问题

目标：消除固定 HTTP 地址、外部关键依赖和明显的动态 HTML 安全薄弱点。

### 1.1 移除帮助页固定服务器 IP

步骤：

- [x] 搜索所有 `http://117.147.213.234:38080`。
- [x] 区分同源链接、CLI `--server` 参数和纯网络诊断示例。
- [x] 同源浏览器链接使用相对 URL。
- [x] CLI 命令使用服务端注入的公开基础地址。
- [x] 在配置中增加或复用明确的公开 URL，例如 `GIT_AI_PUBLIC_BASE_URL`。
- [x] 如果未配置公开 URL，使用当前请求的可信 Origin 或明确回退规则。
- [x] 禁止从任意未经校验的 `Host`/转发头直接生成安装命令。
- [x] 为反向代理场景补充部署说明和测试。

验收标准：

- [x] HTML 中不再包含固定企业 IP。
- [x] HTTP、HTTPS 和反向代理部署生成正确地址。
- [x] 浏览器下载链接和 CLI 命令指向同一个公开服务地址。

### 1.2 强制安全安装链路

步骤：

- [x] 生产配置要求公开地址使用 HTTPS。
- [x] 帮助页面对 HTTP 部署显示显著风险提示，不再默认推荐管道执行命令。
- [x] 安装脚本和二进制下载使用同一可信 HTTPS Origin。
- [x] 保留并验证 SHA256 校验流程。
- [x] 评估安装脚本签名或发布签名；如暂不实现，建立独立后续任务。
- [x] 更新 macOS/Linux 和 Windows 安装示例。

验收标准：

- [x] 生产帮助页不推荐通过 HTTP 下载后直接执行脚本。
- [x] 所有安装示例在实际部署地址上可复制执行。
- [x] 安装文档说明校验和信任边界。

### 1.3 本地托管 Chart.js

步骤：

- [x] 固定当前 Chart.js 版本。
- [x] 把经审核的发布文件放入 `enterprise-server/static/assets/`。
- [x] 把 CDN `<script>` 改为同源静态资源。
- [x] 记录上游版本、许可证和升级步骤。
- [x] 在网络完全离线时验证图表仍可加载。
- [x] 如果继续使用 CDN 作为备选，配置 SRI 和明确 fallback；推荐第一版只使用本地资源。

验收标准：

- [x] Dashboard 不依赖外部 CDN 才能显示图表。
- [x] 依赖版本和许可证可追溯。
- [x] 隔离网络环境功能完整。

### 1.4 修复高风险 HTML sink

步骤：

- [x] 把新 API Key 的展示从 `innerHTML` 改成按钮节点加 `textContent`。
- [x] 审计所有 `innerHTML =`。
- [x] 对每个动态值标记上下文：文本、HTML、属性、URL、CSS 或 JavaScript。
- [x] 文本优先使用 `textContent`。
- [x] 属性优先使用 DOM property 或 `setAttribute()`。
- [x] URL 统一使用 `new URL()` 和允许协议校验。
- [x] 百分比统一转成有限数值并限制到 `0..100`。
- [x] 不把未经校验的数据写入 inline event handler。

验收标准：

- [x] API Key 不经过 HTML 解析。
- [x] 所有动态 sink 已有明确安全处理。
- [x] 添加包含 `<`, `>`, `"`, `'`, `&` 的测试数据时页面只显示文本。

### 1.5 核查服务端安全响应

步骤：

- [x] 确认所有 `/api/admin/*` 服务端执行管理员鉴权。
- [x] 保留 Cookie 的 `HttpOnly`、`SameSite` 和 HTTPS 下的 `Secure`。
- [x] 评估并添加以下响应头：

```text
Content-Security-Policy
X-Content-Type-Options: nosniff
Referrer-Policy
frame-ancestors 或 X-Frame-Options
```

- [x] 第一版 CSP 可以使用 Report-Only 收集违规。
- [x] 记录 inline handler 和 inline script 对严格 CSP 的阻碍，阶段 6 移除后收紧。
- [x] 管理 mutation 继续依赖服务端权限、Origin/CSRF 策略和审计日志。

验收标准：

- [x] 非管理员直接请求管理 API 返回 403。
- [x] 安全头存在且不会破坏当前页面。
- [x] CSP 收紧路径已有明确记录。

### 阶段 1 验证

```bash
node --check enterprise-server/static/dashboard.js
cargo test --manifest-path enterprise-server/Cargo.toml
```

- [x] 离线环境图表正常。
- [x] 帮助页没有固定 IP。
- [x] HTTP/HTTPS 部署地址生成正确。
- [x] 动态字符串安全测试通过。

建议提交拆分：

```text
Use configured enterprise frontend URLs
Serve dashboard charts locally
Harden dashboard dynamic rendering
```

## 阶段 2：建立统一请求层

目标：统一 HTTP 状态、身份过期、JSON 解析、超时、取消和错误展示。

执行记录：[`enterprise-frontend-request-layer.md`](./enterprise-frontend-request-layer.md)

### 2.1 定义统一错误类型

步骤：

- [x] 定义前端错误分类：

```text
AuthExpiredError
PermissionDeniedError
HttpError
InvalidResponseError
NetworkError
TimeoutError
AbortError
```

- [x] 每个错误保留 status、用户可读消息和可选 request ID。
- [x] 不把服务端内部错误堆栈直接显示给用户。

验收标准：

- [x] 调用者可以根据错误类型决定跳转、重试或保留旧数据。
- [x] 错误日志有足够排查信息。

### 2.2 实现 `apiRequest()`

步骤：

- [x] 第一版可以先放在 `dashboard.js` 顶部，阶段 6 再移动到 `api.js`。
- [x] 默认发送 `Accept: application/json`。
- [x] 检查 HTTP status 和 Content-Type。
- [x] 安全处理空 body、非 JSON body 和格式错误 JSON。
- [x] 支持 `AbortSignal`。
- [x] 支持显式超时。
- [x] 401 统一跳转 `/auth/login?return_to=<current-url>`。
- [x] 403 显示明确权限提示，不伪装成空数据。
- [x] GET 仅在网络错误或明确可重试状态下有限退避重试。
- [x] POST、PUT、DELETE 和文件上传默认不重试。
- [x] 保留服务端返回的可读错误消息。

验收标准：

- [x] 页面不再把 401 JSON 当作正常数据。
- [x] 非 JSON 错误响应不会产生二次解析异常。
- [x] 请求可以被栏目切换和超时取消。

### 2.3 迁移所有裸 `fetch()`

迁移顺序：

- [x] 总览、趋势和客户端状态 GET。
- [x] 通用分页和全量分页 GET。
- [x] 用户、部门和 API Key 管理。
- [x] CLI 发布和文件中心。
- [x] 注册页面中的组织、部门请求。

每迁移一组后：

- [x] 搜索是否仍有意外裸 `fetch()`。
- [x] 验证成功、401、403、500、网络断开和取消。

验收标准：

- [x] Dashboard 业务代码不再直接调用裸 `fetch()`。
- [x] 认证页若保留独立请求 helper，行为与 Dashboard 一致。

### 2.4 统一页面错误状态

步骤：

- [x] 定义首次加载错误、后台刷新错误和 mutation 错误的不同展示。
- [x] 首次加载失败显示栏目错误和重试按钮。
- [x] 后台刷新失败保留旧数据并显示“数据可能已过期”。
- [x] mutation 失败保留用户输入和操作上下文。
- [x] 记录“最后尝试刷新”和“最后成功刷新”。

验收标准：

- [x] 错误不会用空表格覆盖有效数据。
- [x] 用户可以从错误状态直接重试。
- [x] 登录过期可以恢复到原页面。

### 阶段 2 验证

- [ ] 使用浏览器拦截分别返回 401、403、429、500、HTML 和空响应（自动响应拦截已覆盖；
  本轮浏览器无法访问本机回环服务）。
- [ ] 使用 Offline 模式验证网络错误（自动响应拦截已覆盖）。
- [ ] 使用 Slow 3G 验证超时和取消（慢请求头、慢响应体和主动取消的自动测试已覆盖）。
- [x] 请求层自动响应拦截矩阵覆盖上述状态、网络、超时和取消场景。
- [x] 注册页内嵌脚本通过独立语法与裸 `fetch()` 所有权检查。

```bash
node --check enterprise-server/static/dashboard.js
cargo test --manifest-path enterprise-server/Cargo.toml
```

建议提交信息：

```text
Centralize enterprise frontend requests
```

## 阶段 3：修复移动端和响应式体验

目标：让 390px 宽度下可以访问全部授权栏目、执行常用操作并安全查看表格。

执行记录：[`enterprise-frontend-responsive.md`](./enterprise-frontend-responsive.md)

### 3.1 增加移动端导航

步骤：

- [x] 在页面头部增加移动端菜单按钮。
- [x] 侧边栏改成可开关的抽屉，而不是 `display: none`。
- [x] 增加遮罩层。
- [x] 点击栏目、遮罩或 Escape 后关闭抽屉。
- [x] 打开后把焦点移入菜单，关闭后恢复到触发按钮。
- [x] 防止抽屉打开时背景滚动。
- [x] 保留管理员栏目权限隐藏。
- [x] 移动端提供可见的用户信息和退出入口。

验收标准：

- [x] 390px 布局提供全部有权访问栏目的抽屉入口。
- [x] 菜单具备完整键盘操作、焦点进入和恢复逻辑。
- [x] 退出登录入口始终可访问。

### 3.2 统一表格横向滚动

步骤：

- [x] 为所有数据表增加统一 `.table-scroll` 容器。
- [x] 避免 `.table-card { overflow: hidden }` 裁剪宽表格。
- [x] 保持表头和单元格最小宽度。
- [x] 评估首列 sticky，仅在不遮挡内容时启用。
- [x] 在小屏上保证操作按钮不会被裁剪。
- [x] 横向滚动容器具有可见焦点和可访问名称。

验收标准：

- [x] 开发者、用户、API Key 等宽表格具有独立横向滚动区域。
- [x] 页面主内容限制横向溢出，宽度由表格滚动区域承接。

### 3.3 调整小屏布局

步骤：

- [x] 390px 下统计卡从两列改为一列或自适应最小宽度。
- [x] Toolbar 控件宽度允许换行并保持标签关联。
- [x] 发布和文件表单按钮使用至少 44px 触控高度。
- [x] 模态框适配软键盘和小屏高度。
- [x] 帮助页代码块复制按钮不遮挡代码。
- [x] 添加 `prefers-reduced-motion`。

验收标准：

- [x] 关键控件满足基本触控尺寸。
- [ ] 页面缩放到 200% 仍可完成主要操作（已进入相同响应式重排路径，待浏览器冒烟）。

### 阶段 3 验证

对以下 viewport 执行全部导航和至少一个管理操作：

```text
390 × 844
768 × 1024
1024 × 768
1440 × 900
```

- [ ] Chrome 响应式模式通过。
- [ ] Safari/WebKit 或真实 iOS 设备至少完成一次冒烟测试。
- [ ] Windows 浏览器至少完成一次表格测试。
- [x] 自动化结构测试覆盖移动导航、键盘行为、全部表格滚动区域和小屏 CSS 约束。

建议提交信息：

```text
Make enterprise dashboard navigation responsive
```

## 阶段 4：执行自动刷新专项优化

目标：消除自动刷新闪烁、请求重叠、状态丢失和图表重建。

本阶段以以下文档为唯一详细执行清单：

[`docs/enterprise/dashboard-auto-refresh-flicker-optimization-task-plan.md`](./dashboard-auto-refresh-flicker-optimization-task-plan.md)

### 4.1 确认前置条件

- [x] 阶段 2 的 `apiRequest()` 已支持 `AbortSignal`。
- [x] 首次、手动和自动刷新可以区分。
- [x] 错误展示支持保留旧数据。

### 4.2 执行专项计划

- [x] 完成专项阶段 0：记录基线。
- [x] 完成专项阶段 1：统一刷新上下文。
- [x] 完成专项阶段 2：表格静默刷新。
- [x] 完成专项阶段 3：复用图表。
- [x] 完成专项阶段 4：稳定总览更新。
- [x] 完成专项阶段 5：防止重叠和过期响应。
- [x] 完成专项阶段 6：补充回归验证。
- [ ] 完成专项阶段 7：浏览器验收。

### 4.3 补充状态保留

- [x] 用户列表自动刷新不清除批量选择。
- [x] 如果已选用户被删除或变为已授权，只移除失效选择。
- [x] 部门层级保持不变。
- [x] 分页页码和 cursor 保持不变。
- [x] 自动刷新不关闭模态框、不清空表单、不改变焦点。

验收标准：

- [ ] 所有动态栏目自动刷新无可见闪烁。
- [ ] 无数据变化时不产生主要 DOM 重建。
- [ ] 请求失败保留旧数据。
- [ ] 旧响应不能覆盖新栏目或新筛选状态。

建议按专项文档拆分提交，不合并为一个超大提交。

## 阶段 5：优化大数据和静态资源性能

目标：降低部门树、下拉选项、静态资源和帮助内容的长期扩展成本。

### 5.1 部门按层加载

执行记录：[`enterprise-frontend-department-loading.md`](./enterprise-frontend-department-loading.md)

步骤：

- [x] 为部门聚合接口增加明确的 `parent_id` 或层级查询语义。
- [x] 根层级只返回根部门和 `has_children`。
- [x] 展开子部门时才请求下一层。
- [x] 非管理员只返回允许查看的部门范围。
- [x] 面包屑使用已加载节点或专用 ancestor 信息。
- [x] 当前层级结果可以短期缓存。
- [x] 创建部门表单使用按组织、关键词搜索接口。
- [x] 不再通过 `fetchAllPaginated()` 获取整棵树。

验收标准：

- [x] 部门页面首屏只发一个有界请求。
- [x] 10,000 个部门下首屏请求量不随总部门数线性增长。
- [ ] 当前层级、面包屑和创建父部门选择行为不变。

### 5.2 替换全量下拉数据

执行记录：[`enterprise-frontend-bounded-options.md`](./enterprise-frontend-bounded-options.md)

依次处理：

- [x] 创建用户的组织选择。
- [x] 创建用户的部门选择。
- [x] 创建部门的组织选择。
- [x] 创建部门的上级部门选择。

推荐方案：

- 组织数量小：有界缓存。
- 部门数量大：关键词搜索、分页或按层选择。

验收标准：

- [x] 不存在达到 5,000 条后静默截断的下拉数据。
- [x] 搜索明确显示 loading、无结果和错误状态。

### 5.3 优化静态资源传输

执行记录：[`enterprise-frontend-static-asset-caching.md`](./enterprise-frontend-static-asset-caching.md)

步骤：

- [x] 为静态资源增加 ETag 或 Last-Modified。
- [x] 对匹配的条件请求返回 304。
- [x] 确认 Gzip/Brotli 压缩层实际启用。
- [x] HTML 保持短缓存或 `no-cache`。
- [x] 版本化 JS/CSS 可以使用更长缓存。
- [x] 如果没有内容 hash，先使用 ETag，不要错误设置不可变长期缓存。
- [x] 记录 Chart.js 和自有资源的缓存策略。

验收标准：

- [x] 第二次加载 JS/CSS 返回 304 或命中缓存。
- [x] 压缩响应大小有记录。
- [x] 发布新版本后不会长期使用旧资源。

### 5.4 减少初始 DOM 和无用脚本

执行记录：[`enterprise-frontend-initial-payload.md`](./enterprise-frontend-initial-payload.md)

步骤：

- [x] 评估帮助页大段内容是否拆成独立页面或按需加载片段。
- [x] 删除未使用的 `currentUserId`、`name`、`email` 全局变量。
- [x] 删除已失效的 helper 和重复 CSS。
- [x] 每个栏目只初始化必要事件和图表。
- [x] 不提前创建隐藏栏目的 Chart 实例。

验收标准：

- [x] 初始 HTML、JS 和 DOM 节点数量有优化前后记录。
- [x] 删除内容不影响深链接和帮助页复制功能。

### 5.5 管理上传体验

执行记录：[`enterprise-frontend-upload-safeguards.md`](./enterprise-frontend-upload-safeguards.md)

步骤：

- [x] 客户端预检查文件数量、文件名、扩展名和已知大小限制。
- [x] 显示单文件和总上传大小。
- [x] 上传期间阻止重复提交。
- [x] 页面离开前对进行中的上传给出提示。
- [x] 如果上传耗时明显，评估可取消上传和进度展示。
- [x] 服务端继续作为最终校验边界。

验收标准：

- [x] 明显无效的发布包在上传前被提示。
- [x] 连续点击不会创建重复发布或重复文件版本。

### 阶段 5 验证

- [x] 记录部门数据 100、1,000、10,000 条时的请求数和响应时间。
- [x] 记录静态资源首次和二次加载传输量。
- [x] 记录 Gzip/Brotli 前后大小。
- [x] 验证所有下拉选择不会静默截断。

建议提交拆分：

```text
Load dashboard departments by level
Bound enterprise form option loading
Cache enterprise static assets efficiently
```

## 阶段 6：移除 inline handler 并拆分模块

目标：消除全局函数和超长脚本，建立可测试的明确模块边界。

### 6.1 建立事件委托

执行记录：[`enterprise-frontend-event-delegation.md`](./enterprise-frontend-event-delegation.md)

步骤：

- [x] 统计静态 HTML 中所有 `onclick`、`onchange` 和其他 inline handler。
- [x] 统计动态模板中生成的 inline handler。
- [x] 为操作按钮增加 `data-action`。
- [x] 参数通过 `dataset` 或受控状态 Map 传递。
- [x] 在栏目容器或 document 上注册一次事件监听器。
- [x] 先迁移分页、导航和简单刷新。
- [x] 再迁移用户、API Key、发布和文件管理操作。
- [x] 最后迁移模态框和帮助页复制按钮。

验收标准：

- [x] HTML 和动态模板中不再出现 inline event handler。
- [x] 业务函数不需要挂到 `window`。
- [x] 包含特殊字符的名称不会破坏事件参数。

### 6.2 删除内联脚本变量

步骤：

- [x] 用 `<script type="application/json" id="dashboard-bootstrap">` 或 `data-*` 提供启动数据。
- [x] JSON 内容必须使用安全序列化，防止 `</script>` 提前结束。
- [x] 删除未使用的 `name`、`email` 和 `currentUserId`。
- [x] 管理员角色可以由服务端渲染栏目或通过安全 bootstrap 数据提供。
- [x] 管理员内容默认隐藏，避免角色判断执行前闪现。

验收标准：

- [x] 页面 head/body 中不再需要可执行内联脚本。
- [x] 非管理员不会收到不必要的管理页面 DOM，或管理内容默认不可见。

### 6.3 提取无 UI 基础模块

建议顺序：

- [x] `api.js`
- [x] `state.js`
- [x] `router.js`
- [x] `pagination.js`
- [ ] `refresh.js`
- [x] `render.js`

要求：

- [ ] 模块不依赖隐式全局变量。
- [ ] 纯逻辑函数可以在 Node 中测试。
- [ ] DOM selector 和 API URL 不散落在多个模块。
- [ ] 循环依赖在 code review 中禁止合入。

验收标准：

- [ ] `dashboard.js` 不再承担所有基础设施职责。
- [ ] 请求、分页、路由和刷新模块拥有独立测试边界。

### 6.4 提取 UI 模块

- [ ] `ui/toast.js`
- [ ] `ui/modal.js`
- [ ] 通用 loading、empty、error renderer。
- [ ] 通用安全文本、属性和 URL helper。

验收标准：

- [ ] 所有栏目使用同一 Toast、模态框和错误组件。
- [ ] UI 模块不直接调用业务 API。

### 6.5 按栏目提取 section 模块

推荐顺序：

1. [ ] 组织、项目、工具等只读表格。
2. [ ] 开发者和部门。
3. [ ] 总览和趋势图表。
4. [ ] 用户和 API Key。
5. [ ] 发布和文件管理。

每个模块必须：

- [ ] 明确声明依赖。
- [ ] 导出 `load()`。
- [ ] 支持 `AbortSignal`。
- [ ] 不读写其他栏目的私有状态。
- [ ] 对需要清理的图表或监听器提供 `dispose()`。

验收标准：

- [ ] `app.js` 只负责初始化、路由和模块调度。
- [ ] 单个栏目修改不需要编辑其他栏目的实现文件。

### 6.6 启用严格 CSP

步骤：

- [ ] 确认没有 inline script、inline style 属性和 inline handler。
- [ ] 把必要样式移入 CSS class。
- [ ] CSP Report-Only 中清理违规。
- [ ] 切换为强制 CSP。
- [ ] 至少限制：

```text
default-src 'self'
script-src 'self'
style-src 'self'
object-src 'none'
base-uri 'self'
frame-ancestors 'none'
```

- [ ] 根据实际图片、字体和连接需求最小化补充。

验收标准：

- [ ] 页面在不使用 `unsafe-inline`、`unsafe-eval` 的 CSP 下工作。
- [ ] 浏览器控制台没有 CSP 违规。

### 阶段 6 验证

```bash
node --check enterprise-server/static/dashboard/app.js
```

- [ ] 所有栏目加载正常。
- [ ] 浏览器前进/后退正常。
- [ ] 所有管理操作正常。
- [ ] CSP 强制模式正常。

建议按基础模块、UI 模块和栏目模块分别提交，禁止一次性移动全部代码。

## 阶段 7：统一 Rust 服务端页面

目标：减少 handler 中重复 HTML/CSS，统一认证页面视觉、安全和测试方式。

### 7.1 提取共享认证样式

步骤：

- [ ] 把 `AUTH_PAGE_STYLES` 移到 `enterprise-server/static/auth.css`。
- [ ] 登录、注册、成功页和 CLI 授权页统一引用。
- [ ] `login.rs` 的旧 token 登录页面也引用共享变量和基础组件。
- [ ] 保留页面特有样式的最小扩展。

验收标准：

- [ ] 颜色、字体、按钮和表单样式只有一个主要来源。
- [ ] 修改品牌样式不需要同时编辑多个 Rust 文件。

### 7.2 统一页面 shell

步骤：

- [ ] 提取共享 head、品牌、卡片和错误区域。
- [ ] 明确评估模板方案：

```text
方案 A：编译期模板（推荐长期方案）
方案 B：受控静态模板 + 安全占位替换
```

- [ ] 记录选择理由、依赖和迁移范围。
- [ ] 首先迁移成功页和 CLI 授权页。
- [ ] 再迁移登录、注册页。
- [ ] 最后处理设备验证和 bundle 页面。

验收标准：

- [ ] 用户输入和服务端数据默认经过 HTML 转义。
- [ ] handler 主要负责数据和响应，不包含大段 CSS。
- [ ] 页面模板可以单独审查。

### 7.3 统一认证页请求逻辑

步骤：

- [ ] 注册组织和部门请求使用共享请求 helper 或行为一致的轻量版本。
- [ ] 增加请求取消，避免快速修改邮箱时旧响应覆盖新结果。
- [ ] 统一 loading、空结果和错误状态。
- [ ] 表单提交期间禁用重复提交。
- [ ] 保留 `autocomplete`、服务端验证和安全 return URL。

验收标准：

- [ ] 快速切换邮箱或组织不会显示过期选项。
- [ ] 登录和注册错误保持用户输入并可恢复。

### 7.4 清理旧页面和重复入口

步骤：

- [ ] 确认 `/login` token 登录与 `/auth/login` 账号登录的真实使用场景。
- [ ] 如果两者都需要，明确页面名称和导航关系。
- [ ] 如果旧入口已废弃，制定兼容跳转和删除计划。
- [ ] 更新文档和测试。

验收标准：

- [ ] 用户不会遇到两个含义不清的登录页面。
- [ ] CLI 授权和 Dashboard 登录路径明确。

### 阶段 7 验证

```bash
cargo test --manifest-path enterprise-server/Cargo.toml
```

- [ ] 登录、注册、return_to、CLI 授权、成功和错误页面全部通过。
- [ ] HTML 转义测试通过。
- [ ] 页面样式在移动端正常。

建议提交信息：

```text
Share enterprise authentication page assets
Unify enterprise server-rendered page templates
```

## 阶段 8：完善功能体验和可访问性

目标：让页面状态可恢复、管理操作更可靠，并满足基础键盘和读屏使用要求。

### 8.1 持久化页面状态

步骤：

- [ ] 将当前栏目保留在 URL。
- [ ] 将时间范围、趋势指标和粒度写入 query 参数。
- [ ] 将排序和必要筛选写入 query 参数。
- [ ] 评估分页 cursor 是否适合写 URL；如果不适合，至少保留页码和筛选。
- [ ] 浏览器前进/后退恢复页面状态。
- [ ] 分享 URL 后可以还原同一视图。

验收标准：

- [ ] 页面刷新后主要筛选不丢失。
- [ ] 无权访问的 section 参数安全回退。

### 8.2 增加搜索和筛选

根据数据量和使用频率依次增加：

- [ ] 用户：姓名、邮箱、授权状态。
- [ ] 开发者：姓名、邮箱、部门。
- [ ] 部门：编号、名称。
- [ ] 项目：项目名、分支。
- [ ] API Key：名称、状态、过期状态。
- [ ] 文件和发布：版本、状态。

要求：

- [ ] 大数据搜索下推到服务端。
- [ ] 输入请求有 debounce 和取消。
- [ ] 搜索状态进入 URL。
- [ ] 清空筛选可恢复默认列表。

验收标准：

- [ ] 不通过在浏览器中全量加载数据实现搜索。
- [ ] 搜索结果分页稳定。

### 8.3 统一高风险确认

步骤：

- [ ] 替换浏览器原生 `confirm()`。
- [ ] 确认弹窗明确显示对象、版本、后果和不可逆性。
- [ ] 删除、撤销、切换 latest 和公开文件设置使用不同风险级别。
- [ ] 确认按钮防重复提交。
- [ ] 操作成功后显示稳定、可读的结果。
- [ ] 服务端记录操作者和对象。

验收标准：

- [ ] 所有高风险操作可用键盘完成。
- [ ] 连续点击不会产生重复 mutation。
- [ ] 用户可以明确识别正在操作的对象。

### 8.4 完善动态状态

步骤：

- [ ] Toast 容器增加 `aria-live="polite"`。
- [ ] 错误使用 `role="alert"`。
- [ ] loading 容器使用 `aria-busy`。
- [ ] 后台刷新不反复播报整个表格。
- [ ] “最后成功刷新”和 stale 状态可见。
- [ ] 每个栏目错误状态提供重试按钮。

验收标准：

- [ ] 读屏器可以感知重要操作结果。
- [ ] 后台刷新不会抢焦点或造成重复播报。

### 8.5 完善模态框

步骤：

- [ ] 增加 `role="dialog"` 和 `aria-modal="true"`。
- [ ] 标题通过 `aria-labelledby` 关联。
- [ ] 打开时保存当前焦点。
- [ ] 把焦点移到首个可操作控件。
- [ ] 实现 Tab focus trap。
- [ ] Escape 关闭非提交状态弹窗。
- [ ] 关闭后恢复焦点。
- [ ] 上传或提交中防止误关闭。

验收标准：

- [ ] 仅使用键盘可以打开、填写、提交和关闭弹窗。
- [ ] 焦点不会落到遮罩后的页面。

### 8.6 提供图表替代内容

步骤：

- [ ] 为每个图表提供文字摘要。
- [ ] 提供可展开的数据表。
- [ ] 图例不只依靠颜色区分。
- [ ] 颜色对比度达到基础可读标准。
- [ ] `prefers-reduced-motion` 下禁用非必要动画。

验收标准：

- [ ] 不查看 canvas 也可以获取关键趋势和数值。
- [ ] 键盘和读屏用户可以访问数据表。

### 阶段 8 验证

- [ ] 仅键盘完成登录、导航、筛选、打开弹窗和关闭弹窗。
- [ ] 使用 VoiceOver、NVDA 或等价读屏工具完成一次冒烟测试。
- [ ] 浏览器缩放 200% 后主要功能可用。
- [ ] 使用 `prefers-reduced-motion` 验证动画减少。

建议提交拆分：

```text
Persist enterprise dashboard view state
Improve enterprise admin action feedback
Make enterprise dashboard interactions accessible
```

## 阶段 9：建立前端自动化测试

目标：覆盖请求层、刷新竞态、分页、权限展示、管理操作和可访问性关键路径。

### 9.1 纯逻辑单元测试

优先使用 Node 内置 `node:test`，避免仅为少量纯函数引入大型测试框架。

覆盖：

- [ ] URL 和 section 解析。
- [ ] 时间范围参数。
- [ ] cursor 分页状态。
- [ ] 数值格式和百分比限制。
- [ ] 错误类型和响应解析。
- [ ] HTML/属性/URL 安全 helper。
- [ ] 刷新模式和 request token。

命令示例：

```bash
node --test enterprise-server/static/dashboard/**/*.test.js
```

验收标准：

- [ ] 纯逻辑不依赖真实浏览器 DOM。
- [ ] 单元测试可以在 CI 中快速运行。

### 9.2 DOM 行为测试

覆盖：

- [ ] 相同内容不重复替换 DOM。
- [ ] 自动刷新不显示 loading。
- [ ] Toast live region。
- [ ] 模态框焦点和 Escape。
- [ ] 用户批量选择保留。
- [ ] 非管理员栏目隐藏。
- [ ] 事件委托参数读取。

实施要求：

- [ ] 先评估轻量 DOM 环境。
- [ ] 新依赖必须记录理由和锁文件。
- [ ] 不编写只断言源码字符串存在的脆弱测试。

### 9.3 浏览器端 E2E

建议建立最小 Playwright 测试层，覆盖高价值路径：

- [ ] 登录并进入总览。
- [ ] 移动端打开导航并切换栏目。
- [ ] 部门层级导航。
- [ ] 用户列表筛选和批量选择。
- [ ] 自动刷新保留状态。
- [ ] 401 返回登录页。
- [ ] 模态框键盘行为。
- [ ] 管理 mutation 使用隔离测试数据。

实施步骤：

- [ ] 建立独立前端测试目录和 package lock。
- [ ] 只安装 Chromium 作为第一版。
- [ ] 测试服务使用独立数据库或可清理 fixture。
- [ ] CI 保存失败截图、trace 和网络日志。
- [ ] 后续再增加 WebKit/Firefox 冒烟测试。

验收标准：

- [ ] P0 用户流程在 CI 中自动执行。
- [ ] 失败能够从截图和 trace 定位。

### 9.4 静态检查

步骤：

- [ ] 所有 JavaScript 使用统一格式和 lint 规则。
- [ ] 增加 HTML 可访问性检查。
- [ ] 增加 CSS 基础检查或至少禁止重复和明显无效规则。
- [ ] 把命令接入 Taskfile 和 CI。

建议命令入口：

```text
task frontend:check
task frontend:test
task frontend:e2e
```

验收标准：

- [ ] 本地和 CI 使用相同命令。
- [ ] 新增前端代码无法绕过基础检查。

建议提交信息：

```text
Add enterprise frontend unit tests
Add enterprise dashboard browser smoke tests
```

## 阶段 10：全量回归、性能对比和发布

目标：确认优化没有破坏角色权限、认证流程、管理功能和部署兼容性。

### 10.1 自动检查

```bash
node --check enterprise-server/static/dashboard/app.js
node --test enterprise-server/static/dashboard/**/*.test.js
cargo test --manifest-path enterprise-server/Cargo.toml
task lint
```

如果已经接入 Taskfile：

```bash
task frontend:check
task frontend:test
task frontend:e2e
```

- [ ] 全部命令通过。
- [ ] 失败项已修复，不使用跳过规避真实回归。

### 10.2 页面验收矩阵

| 页面 | 管理员 | 开发者 | 390px | 键盘 | 401 恢复 | 自动刷新 |
| --- | --- | --- | --- | --- | --- | --- |
| 数据总览 | [ ] | [ ] | [ ] | [ ] | [ ] | [ ] |
| 趋势分析 | [ ] | [ ] | [ ] | [ ] | [ ] | [ ] |
| 组织 | [ ] | 不适用 | [ ] | [ ] | [ ] | [ ] |
| 部门 | [ ] | [ ] | [ ] | [ ] | [ ] | [ ] |
| 开发者 | [ ] | [ ] | [ ] | [ ] | [ ] | [ ] |
| 项目 | [ ] | [ ] | [ ] | [ ] | [ ] | [ ] |
| AI 工具 | [ ] | [ ] | [ ] | [ ] | [ ] | [ ] |
| 用户管理 | [ ] | 不适用 | [ ] | [ ] | [ ] | [ ] |
| API 密钥 | [ ] | 不适用 | [ ] | [ ] | [ ] | [ ] |
| CLI 发布 | [ ] | 不适用 | [ ] | [ ] | [ ] | [ ] |
| 文件中心 | [ ] | 不适用 | [ ] | [ ] | [ ] | [ ] |
| 帮助页 | [ ] | [ ] | [ ] | [ ] | 不适用 | 不适用 |

### 10.3 认证页面验收

- [ ] 账号登录。
- [ ] 注册及组织/部门选择。
- [ ] 登录和注册 return_to。
- [ ] CLI 授权同意与取消。
- [ ] Token/API Key 旧登录入口。
- [ ] 登出和 Cookie 清理。
- [ ] 成功、错误和会话过期页面。

### 10.4 管理操作验收

- [ ] 创建用户。
- [ ] 删除用户。
- [ ] 单个和批量授权 Git 追踪上传。
- [ ] 创建和撤销 API Key。
- [ ] 创建部门和选择上级部门。
- [ ] 上传完整 CLI 发布包。
- [ ] 切换 latest。
- [ ] 上传、发布、删除和设置普通文件。
- [ ] 重复点击不会产生重复 mutation。

### 10.5 性能验收

与阶段 0 对比：

- [ ] 初始 HTML 大小。
- [ ] 初始 JS/CSS 传输大小。
- [ ] 初次加载请求数。
- [ ] 二次加载缓存命中。
- [ ] 总览首次可用时间。
- [ ] 自动刷新 Layout/Paint。
- [ ] 部门页 10,000 条数据下的请求数和响应时间。
- [ ] 页面隐藏期间的请求数。

最低验收目标：

- [ ] 自动刷新不再导致整页或整表闪烁。
- [ ] 部门首屏不随总部门数线性拉取所有页。
- [ ] 二次加载静态资源可命中缓存或返回 304。
- [ ] 页面隐藏时不持续产生无意义轮询。

### 10.6 发布和监控

步骤：

- [ ] 使用测试环境完成管理员和开发者验收。
- [ ] 如果部署系统支持，先灰度到少量用户。
- [ ] 监控前端 401、403、429、5xx 和请求超时。
- [ ] 记录发布前后 Dashboard API 请求量。
- [ ] 保留旧静态资源版本，确保短期回滚。
- [ ] 更新运维和用户文档。

验收标准：

- [ ] 发布后没有异常错误率增长。
- [ ] 无权限绕过或认证回归。
- [ ] 回滚步骤经过验证。

## 完成定义

只有满足以下全部条件，前端总体优化任务才算完成：

- [ ] 移动端可以访问所有有权使用的栏目和退出入口。
- [ ] 帮助页不硬编码服务器 IP，生产安装链路使用可信 HTTPS。
- [ ] Chart.js 可以在隔离网络中加载。
- [ ] 所有 Dashboard 请求经过统一请求层。
- [ ] 401、403、网络错误和非 JSON 响应有一致行为。
- [ ] 自动刷新无闪烁、无请求重叠、无旧响应覆盖。
- [ ] 用户批量选择、分页、筛选和部门层级不会被后台刷新意外清除。
- [ ] 部门页不再周期性全量拉取整个组织树。
- [ ] Dashboard 不再依赖全局 inline handler。
- [ ] 核心脚本已经按基础设施、UI 和栏目拆分。
- [ ] 登录、注册和授权页面共享基础样式和页面 shell。
- [ ] 高风险管理操作防止重复提交并具有明确确认。
- [ ] 模态框、Toast、loading 和图表具备基础可访问性。
- [ ] 静态资源具有正确缓存、压缩和版本更新策略。
- [ ] 前端单元测试和关键浏览器 E2E 已接入 CI。
- [ ] 管理员和开发者完整验收矩阵通过。
- [ ] PR 描述包含问题、设计、兼容性、测试结果和优化前后对比。

## 回滚与兼容性

- 本计划允许每个阶段独立回滚，不应依赖一次性整体重写。
- 阶段 1 的公开 URL 配置必须提供清晰默认值和迁移说明。
- 请求层改造不得改变服务端 API 协议。
- 移动导航改造必须保留桌面侧边栏行为。
- 自动刷新出现回归时可以临时关闭静默模式，但不删除手动刷新能力。
- 部门按层加载如果需要新 API，旧接口应保留一个兼容周期。
- ES Modules 拆分前确认最低支持浏览器版本。
- CSP 应从 Report-Only 逐步切换，避免直接阻断生产页面。
- 静态资源缓存策略必须保证发布新版本后能够失效。
- 认证页面模板改造不得改变 Cookie、return_to 和 CLI 授权协议。
- E2E 测试数据必须与生产数据隔离。

## 框架升级判断标准

完成原生模块拆分后再评估是否引入 Vue、React 或其他框架。只有出现以下多个条件时才建议启动框架迁移：

- 栏目之间存在大量复杂共享状态。
- 需要实时推送、复杂筛选、拖拽或多步骤工作流。
- DOM 增量更新逻辑仍然大量重复。
- 团队已经具备稳定的 Node 构建、依赖和发布能力。
- 前端测试与组件复用收益可以覆盖迁移成本。

如果当前需求仍以数据表格、图表和简单管理表单为主，原生 ES Modules、统一请求层、事件委托和清晰状态边界已经足够。
