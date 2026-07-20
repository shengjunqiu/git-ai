# Enterprise Dashboard 初始载荷与 DOM 优化记录

日期：2026-07-20

## 目标

阶段 5.4 聚焦减少 Dashboard 首次响应中的非必要 HTML 和启动变量，同时保持
帮助页深链接、命令复制、权限校验和图表加载行为不变。

## 实现

- Dashboard 主响应不再内嵌约 24 KB 的帮助内容，只保留带 `aria-busy` 的加载占位。
- 首次进入帮助栏目时，通过受 Dashboard 认证保护的
  `GET /api/v1/dashboard/help` 获取渲染后的帮助片段。
- 帮助片段只请求并提交一次。停留在帮助栏目时，后台刷新不会重复下载或滚动页面。
- `#help-*` 深链接在片段加载完成后恢复定位，复制命令仍使用原有安全复制逻辑。
- 删除未使用的 `currentUserId`、`name`、`email` 内联变量，以及只为这些变量服务的
  Rust JSON 字符串序列化 helper。
- 审核 Chart 初始化位置：三个 Chart 实例都只在总览或趋势 loader 获得数据后创建，
  不会为隐藏栏目预先创建实例。
- 审核 CSS 重复 selector：重复项均属于响应式断点覆盖或关键帧，不是可安全删除的
  完全重复规则，因此本轮不做高风险的机械合并。

## 前后对比

测量使用修改前 `HEAD` 内容和修改后由模板拆分逻辑生成的初始 shell；Gzip 使用相同
Node zlib 设置。动态用户名和静态资源哈希会让实际响应字节数产生少量变化。静态标签
数量用于近似比较初始 DOM 元素规模，不包含浏览器运行时创建的节点。

| 指标 | 优化前 | 优化后 | 变化 |
| --- | ---: | ---: | ---: |
| 初始 HTML 原始大小 | 51,859 B | 27,998 B | -46.0% |
| 初始 HTML Gzip | 9,497 B | 5,055 B | -46.8% |
| 初始 HTML 元素标签数 | 779 | 399 | -48.8% |
| `dashboard.js` 原始大小 | 123,573 B | 124,698 B | +0.9% |
| `dashboard.js` Gzip | 25,316 B | 25,602 B | +1.1% |
| `dashboard.css` 原始大小 | 32,964 B | 32,964 B | 0 |
| `dashboard.css` Gzip | 6,584 B | 6,584 B | 0 |

按需帮助片段为 23,867 B，Gzip 后约 4,774 B，包含 383 个元素标签。默认进入总览、
趋势或管理栏目时不再传输和解析这些内容。

## 验证

```bash
node --check enterprise-server/static/dashboard.js
node --test enterprise-server/static/*.test.cjs
cargo test --manifest-path enterprise-server/Cargo.toml
```

结果：

- JavaScript 语法检查通过。
- 前端行为测试 23/23 通过。
- Enterprise Server 测试 172/172 通过。
- 帮助片段测试覆盖单次请求、无效响应可重试、深链接定位和后台刷新不重复滚动。
- Rust 模板测试覆盖 HTTPS/HTTP 安全提示、动态文本转义、资源版本和无残留占位符。

## 兼容性

- `/me?section=help` 入口保持不变。
- `#help-quick`、`#help-install` 等锚点保持不变。
- 帮助接口复用 Dashboard Cookie、Bearer Token 和 API Key 的现有认证提取器。
- 主 HTML 和帮助接口都使用 `Cache-Control: no-cache`，公开服务器地址变化后不会长期
  使用旧帮助命令。
