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
