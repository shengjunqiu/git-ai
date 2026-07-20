# Enterprise 前端阶段 2：统一请求层

记录日期：2026-07-20

对应计划：[`enterprise-frontend-optimization-task-plan.md`](./enterprise-frontend-optimization-task-plan.md)

## 1. 实施范围

阶段 2 在不引入前端构建工具的前提下，为 Dashboard 和注册页建立一致的请求语义：

- `enterprise-server/static/dashboard.js` 顶部提供 Dashboard 统一 `apiRequest()`。
- `enterprise-server/src/handlers/auth_pages.rs` 中的注册页保留独立 helper，行为与
  Dashboard 对齐；阶段 6/7 再随模块和服务端模板拆分复用。
- Dashboard 的业务读取、分页、管理 mutation 和文件上传不再直接调用裸 `fetch()`。
- 注册页的组织、部门联动请求不再直接解析 `response.json()`。
- `enterprise-server/static/dashboard.css` 增加栏目错误和过期数据状态。

## 2. 错误协议

所有请求错误继承 `ApiRequestError`，并保留 `status`、用户可读 `message` 和可选的
`requestId`：

| 类型 | 场景 | 页面行为 |
| --- | --- | --- |
| `AuthExpiredError` | HTTP 401 | 跳转登录页并在 `return_to` 中保留当前路径、查询和 hash |
| `PermissionDeniedError` | HTTP 403 | 显示明确权限提示 |
| `HttpError` | 其他非成功 HTTP 状态 | 显示安全的服务端消息或 HTTP 状态 |
| `InvalidResponseError` | 成功响应为空、非 JSON 或 JSON 格式错误 | 显示响应无效，不进行二次解析 |
| `NetworkError` | 离线、DNS 或连接错误 | 提示检查网络 |
| `TimeoutError` | 请求头或响应体超过显式超时 | 提示稍后重试 |
| `AbortError` | 栏目切换或调用者主动取消 | 静默结束，不显示失败提示 |

5xx 响应不会把服务端内部错误或堆栈展示给用户。Dashboard 会把错误类型、HTTP 状态和
request ID 写入开发者控制台，页面错误信息只追加可用于服务端排查的 request ID。

## 3. 请求行为

- 默认发送 `Accept: application/json`。
- 同时检查 HTTP status 和 `Content-Type`，通过 `response.text()` 安全处理空响应、
  HTML 响应和格式错误 JSON。
- HTTP 204 返回 `null`；其他空的 2xx 响应视为无效响应。
- Dashboard 默认超时 15 秒，注册页默认超时 10 秒，文件和 CLI 发布上传显式使用
  120 秒。
- GET 最多退避重试一次，首轮等待 250 毫秒；仅重试网络错误、超时和
  429/502/503/504。
- POST、PUT、DELETE 和上传默认不重试。
- 栏目切换会取消上一栏目仍在执行的请求；注册页新的邮箱或组织查询会取消上一轮联动请求。
- URL 中的动态 ID、slug、version、email 和查询参数均在对应组件中编码。

## 4. 页面错误状态

Dashboard 区分三种失败路径：

1. 首次加载失败：栏目顶部显示错误横幅、request ID 和“重试”按钮。
2. 后台刷新失败：保留已渲染内容，刷新指示变为过期状态，并提示“数据可能已过期”。
3. mutation 失败：模态框、输入值、文件选择和操作上下文保持不变，只显示失败原因。

顶部刷新状态同时记录“最后成功”和“最后尝试”。成功加载过的表格在刷新期间不会再次被
“加载中”占位行覆盖。

## 5. 迁移结果

已迁移的 Dashboard 请求包括：

- 总览、趋势、agent 对比和客户端状态。
- 通用游标分页、组织、部门、开发者、项目和工具。
- 用户列表、Git 追踪授权、用户创建和删除。
- 部门列表、组织/上级部门选择和部门创建。
- API Key 列表、创建和撤销。
- CLI 版本/资产读取、发布和 latest 切换。
- 文件中心读取、上传、发布、删除和设置。

代码搜索结果中，Dashboard 和注册页各只保留 `apiRequest()` 内部的一处 `fetch()`。

## 6. 故障验证矩阵

可重复测试：

```bash
node --check enterprise-server/static/dashboard.js
node --test \
  enterprise-server/static/dashboard-api.test.cjs \
  enterprise-server/static/auth-page-api.test.cjs
cargo test --manifest-path enterprise-server/Cargo.toml
```

Node 请求拦截覆盖：

| 场景 | 断言 |
| --- | --- |
| 401 | `AuthExpiredError`、status/request ID、原页面回跳地址 |
| 403 | `PermissionDeniedError` 和服务端可读权限消息 |
| 429 | GET 只重试一次 |
| 500 | 不展示内部错误，保留 request ID |
| HTML、格式错误 JSON、空 2xx | `InvalidResponseError` |
| 离线 | `NetworkError` |
| 慢请求头、慢响应体 | `TimeoutError` |
| 调用者取消 | `AbortError` |
| POST 网络错误 | 不重试 |

本轮结果：JavaScript 语法检查通过；请求层 8 个测试全部通过；Enterprise Server
170 个测试全部通过、0 个失败。编译仍输出基线中已有的 62 条 warning，本阶段没有新增或
处理这些后端 warning。

本轮尝试用应用内浏览器打开 `127.0.0.1` 时被浏览器安全策略拦截；Chrome 中未安装可用的
ChatGPT Chrome Extension。因此浏览器 DevTools 的 Offline/Slow 3G 手工复核仍保留为
环境允许时的补充验证，不用它替代上述已纳入仓库的自动响应拦截回归测试。

## 7. 后续边界

- 阶段 4 继续处理自动刷新期间的批量选择、输入状态、请求竞态和图表静默更新。
- 阶段 6 把 Dashboard helper 移到 `dashboard/api.js`，并移除全局函数和 inline
  handler。
- 阶段 7 统一认证页模板后，再消除注册页独立 helper 的代码重复。
