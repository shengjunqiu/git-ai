# Enterprise 前端阶段 1 安全审计

日期：2026-07-20

## 公开地址与安装信任边界

- `BASE_URL` 是 OAuth、Dashboard 帮助页、CLI `--server` 参数和发布下载的唯一
  可信公开 Origin。服务端不从 `Host`、`Forwarded` 或 `X-Forwarded-*` 生成
  安装命令。
- 配置只接受无凭据、路径、query 和 fragment 的 `http(s)` Origin。非回环
  HTTP 默认拒绝启动；`ALLOW_INSECURE_PUBLIC_URL=true` 只用于已明确接受风险的
  隔离开发环境。
- 浏览器注册、登录和扩展下载链接使用同源相对 URL；需要复制到 CLI 的命令
  使用服务端注入并进行 HTML 转义的 `BASE_URL`。
- macOS、Linux 和 Windows 示例先从同一 Origin 下载安装脚本和
  `SHA256SUMS`，验证安装脚本哈希后再执行。发布生成流程还会把二进制哈希固定
  到安装脚本中。

待办 `ENTERPRISE-SEC-RELEASE-SIGNING`：为 `SHA256SUMS` 和发布清单增加离线
签名、密钥轮换、撤销和客户端验签。SHA256 能检测内容是否改变，但只有在校验
清单来自可信 HTTPS Origin 时才提供完整性保证，不能替代发布签名。

## 动态 HTML sink

`enterprise-server/static/dashboard.js` 的 `innerHTML` 写入按上下文审计如下：

| 上下文 | 来源与处理 |
|---|---|
| 静态结构 | 加载态、空态和 modal 骨架只包含仓库内常量。 |
| 文本节点 | API 返回的姓名、邮箱、组织、部门、版本、文件名、错误消息等在进入 HTML 模板前使用 `escapeHtml()`；纯状态文本优先写入 `textContent`。 |
| HTML 属性 | 表单 `value` 等使用 `escapeAttribute()`；布尔属性先严格转换为布尔值。 |
| inline handler 参数 | 当前遗留 handler 的动态字符串统一经 `jsString()` 生成 JSON 字符串；ID 不再直接拼入引号。阶段 6 移除 inline handler。 |
| URL | fetch path 参数使用 `encodeURIComponent()`；复制下载链接通过 `new URL()` 解析，并只允许与页面同源的 `http:`/`https:`。 |
| CSS 百分比 | 所有百分比先转为有限数值，再限制到 `0..100`，才写入 `style.width` 或显示文本。 |
| API Key | 新密钥通过 `createElement()`、事件监听器和 `textContent` 创建，不经过 HTML 解析；复制值保存在元素 `dataset` 中。 |

测试模板身份包含 `<`, `>`, `"`, `'`, `&`，断言输出只包含转义文本。严格 CSP
仍受 `dashboard.html` 的 inline script、inline style 和 inline handler 阻碍；
当前使用 Report-Only 并临时允许 `unsafe-inline`，阶段 6 完成事件绑定和模块拆分
后移除例外并切换为强制策略。

## 服务端响应与权限

- 路由审计确认 `/api/admin/*` 的 handler 均提取 `AdminGuard`；该 guard 在身份
  不是 owner/admin 时返回 403。管理 mutation 的业务 handler继续写入现有
  audit log。
- Cookie 登录保留 `HttpOnly; SameSite=Lax`，当 `BASE_URL` 为 HTTPS 时附加
  `Secure`。
- cookie-authenticated 的 `POST`、`PUT`、`PATCH` 和 `DELETE /api/admin/*`
  必须携带与 `BASE_URL` 完全同 Origin 的 `Origin`。Bearer/API key 不使用环境
  Cookie，因此不强制浏览器 Origin。
- 全局响应增加：
  - `Content-Security-Policy-Report-Only`
  - `X-Content-Type-Options: nosniff`
  - `Referrer-Policy: no-referrer`
  - `X-Frame-Options: DENY`
- Report-Only CSP 已声明 `frame-ancestors 'none'`；`X-Frame-Options` 在 CSP
  转为强制前提供实际的防嵌入保护。

## 反向代理

反向代理只负责 TLS 终止并转发到 API。`BASE_URL` 必须显式写成用户实际访问的
HTTPS Origin（包括非默认端口），代理传入的 host/forwarded headers 不参与
公开 URL 生成。部署后应同时验证：

```bash
curl -I https://git-ai.example.com/me
curl -I https://git-ai.example.com/static/assets/vendor/chart.js/chart.umd.js
```

两条响应都应包含上述安全头；Chart.js 请求必须命中同源地址，浏览器网络面板
不应出现 CDN 请求。
