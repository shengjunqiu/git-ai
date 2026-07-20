# Enterprise Dashboard 阶段 5.3：静态资源缓存与压缩执行记录

## 范围

本次改造覆盖 Dashboard HTML、`dashboard.css`、`dashboard.js` 和本地 Chart.js 的缓存验证、
版本失效与传输压缩，不改变 API 业务协议。

## 缓存策略

Dashboard 模板在渲染时读取三个首屏资源的实际内容并计算 SHA-256，将完整哈希作为 `v`
查询参数写入资源 URL：

```text
/static/dashboard.css?v=<sha256>
/static/dashboard.js?v=<sha256>
/static/assets/vendor/chart.js/chart.umd.js?v=<sha256>
```

响应策略：

| 资源 | 条件 | Cache-Control |
| --- | --- | --- |
| `/me` Dashboard HTML | 所有响应，包括登录重定向 | `no-cache` |
| 静态 HTML | 即使携带匹配版本 | `no-cache` |
| CSS、JS 和其他静态资源 | `v` 与当前内容 SHA-256 完全一致 | `public, max-age=31536000, immutable` |
| CSS、JS 和其他静态资源 | 无 `v`、旧版本或错误版本 | `public, max-age=0, must-revalidate` |

资源内容在部署期间变化时，旧 URL 不会继续获得 immutable：服务端会发现查询参数与当前内容
不匹配并退回强制重验证。下一次加载 `no-cache` 的 Dashboard HTML 会获得新哈希 URL。

所有静态资源都返回基于源内容的弱 SHA-256 ETag。弱校验器允许 gzip、Brotli 和 identity
传输表示共享相同的源内容身份；匹配 `If-None-Match` 时返回无响应体的 `304 Not Modified`。
可压缩资源的 200 和 304 响应都保留 `Vary: Accept-Encoding`。

## 压缩策略

全局 HTTP 响应层显式启用 gzip 和 Brotli，压缩级别设为 5。压缩白名单只接受不少于
256 字节的文本、JavaScript、JSON、XML、SVG 和 WebAssembly 响应；安装包、发布产物、
普通文件等二进制响应不会被动态重复压缩。范围响应和已经压缩的响应仍由中间件自动跳过。

首屏三个静态资源的中间件实测结果：

| 资源 | Identity | gzip | gzip 减少 | Brotli | Brotli 减少 |
| --- | ---: | ---: | ---: | ---: | ---: |
| `dashboard.css` | 32,964 B | 6,684 B | 79.7% | 6,418 B | 80.5% |
| `dashboard.js` | 123,573 B | 25,445 B | 79.4% | 23,436 B | 81.0% |
| Chart.js 4.4.7 | 205,615 B | 70,499 B | 65.7% | 66,604 B | 67.6% |
| 合计 | 362,152 B | 102,628 B | 71.7% | 96,458 B | 73.4% |

首次加载使用 Brotli 时，这三个资源传输约 96 KB；相同版本在一年有效期内由浏览器直接复用。
无版本调用仍可通过 ETag 重验证并获得无响应体的 304。

## Chart.js 和自有资源

- Chart.js 仍是仓库内固定的 4.4.7 版本，来源和上游 SHA-256 记录保持在 vendor README。
- Chart.js、Dashboard CSS 和 Dashboard JS 都使用各自实际文件内容生成 URL 版本。
- 不依赖文件名是否包含版本号，不对未匹配内容哈希的 URL 设置 immutable。
- 文件读取仍经过原有路径净化和静态目录解析；非法路径、缺失文件和读取失败继续返回 404。

## 验证

已执行：

```bash
cargo fmt --manifest-path enterprise-server/Cargo.toml
node --check enterprise-server/static/dashboard.js
node --test enterprise-server/static/*.test.cjs
cargo test --manifest-path enterprise-server/Cargo.toml \
  dashboard_static_assets_revalidate_version_and_compress -- --nocapture
DATABASE_URL=postgresql://gitai:gitai@localhost:5433/gitai_enterprise \
  cargo test --manifest-path enterprise-server/Cargo.toml
```

结果：

- 中间件专项测试确认 ETag、304、版本匹配、旧版本回退、gzip、Brotli、`Vary` 和二进制跳过。
- Enterprise 完整测试 172/172 通过。
- 静态前端行为测试 20/20 通过。
- `git diff --check` 通过。
