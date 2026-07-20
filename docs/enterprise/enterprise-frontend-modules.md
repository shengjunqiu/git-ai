# Enterprise Dashboard 基础模块拆分

## 阶段 6.3 批次 1：请求层

Dashboard 入口已切换为 ES module，并将请求基础设施从
`enterprise-server/static/dashboard.js` 提取到
`enterprise-server/static/dashboard/api.js`。

### 模块边界

- `api.js` 是叶子模块，不依赖 Dashboard 状态、路由、分页、刷新或 UI。
- `createApiClient({ fetchImpl, location })` 显式接收网络与跳转依赖。
- 请求错误类型、GET 重试、超时、调用方取消、请求 ID 和安全错误消息行为保持不变。
- `dashboard.js` 是唯一初始化入口，负责创建 API client 并启动页面。
- HTML 使用单个 `type="module"` 入口；被导入模块不自行注册监听器或启动定时器。

后续批次必须保持单向依赖：

```text
dashboard.js
  ├── api.js
  ├── state.js
  ├── render.js
  ├── refresh.js
  ├── router.js
  └── pagination.js
```

刷新模块通过注入的栏目 loader registry 调用业务栏目；分页模块通过注入的 reload
callback 触发重新加载。基础模块不得反向导入栏目实现，以避免循环依赖。

### 缓存与部署

入口 `dashboard.js` 继续使用服务端 SHA-256 版本参数和 immutable 缓存。
浏览器对相对导入的 `/static/dashboard/api.js` 使用现有的 ETag 和
`must-revalidate` 策略，因此模块更新不会被旧缓存长期遮蔽。

### 验证

```bash
node --check enterprise-server/static/dashboard.js
node --test enterprise-server/static/*.test.cjs
cargo fmt --manifest-path enterprise-server/Cargo.toml -- --check
cargo test --manifest-path enterprise-server/Cargo.toml
git diff --check
```

前端测试覆盖独立模块导入、显式依赖注入、请求错误分类、重试、超时和取消。
服务端测试覆盖嵌套模块的静态资源响应、压缩、模板加载及缓存策略。

## 阶段 6.3 批次 2：状态与通用渲染

- `state.js` 提供不可变的栏目/刷新常量，以及每次初始化均相互隔离的
  `createDashboardState()`。
- 跨栏目共享的当前栏目、刷新任务、排队中的手动刷新、成功栏目和刷新时间统一通过
  `appState` 属性访问，避免对 ES module imported binding 重新赋值。
- 部门层级、分页、选项搜索、移动导航、图表和开发者详情等状态仍由对应功能持有。
- `render.js` 提供数值/时间格式化、安全文本和属性转义，以及接收明确 element 参数的
  幂等 DOM 更新 helper。
- `escapeHtml()` 不再通过临时 DOM 节点工作；`fmtTimeAgo()` 支持注入当前时间，使纯逻辑
  测试可重复。

专项 Node 测试覆盖状态实例隔离、非法栏目降级、不可变常量、确定性时间格式化、安全
转义和不变内容零写入。

## 阶段 6.3 批次 3：路由

- `router.js` 从 `state.js` 单向导入栏目白名单、默认栏目和管理员栏目常量，并通过
  `createDashboardRouter({ isAdmin, location, history })` 显式接收角色和浏览器依赖。
- 路由模块只负责访问判断、URL 中的栏目解析和 History API 写入；DOM 激活、
  `appState.currentSection` 更新、栏目加载、移动导航和生命周期监听仍由入口编排。
- 普通栏目导航继续使用 `pushState`，首次非法或越权栏目修正继续使用
  `replaceState`；默认栏目写入会删除 `section`，同时保留其他查询参数并清除 hash。
- 初次加载与 `popstate` 的既有边缘行为保持不变：只读取路由不会改写 history，
  `?section=` 和 `?section=overview` 不会被入口主动规范化，历史记录中的非法栏目只让
  UI 降级到默认栏目。

专项 Node 测试覆盖精确白名单匹配、管理员栏目保护、请求参数读取、其他查询参数保留、
默认栏目参数删除、hash 清除和 `pushState` / `replaceState` 选择。
