# Enterprise Dashboard 阶段 4：自动刷新专项执行记录

记录日期：2026-07-20

详细清单：
[`dashboard-auto-refresh-flicker-optimization-task-plan.md`](./dashboard-auto-refresh-flicker-optimization-task-plan.md)

总体计划：
[`enterprise-frontend-optimization-task-plan.md`](./enterprise-frontend-optimization-task-plan.md)

## 1. 专项基线

开始提交：`5f37a10 Make enterprise dashboard navigation responsive`

开始时工作区干净。本专项主要修改 `enterprise-server/static/dashboard.js`；按阶段需要同步
调整 Dashboard HTML、专项清单、执行记录和 Node 测试。

现有代码和阶段 0 浏览器基线确认：

- 只有一个 60 秒定时器，入口为
  `startAutoRefresh() -> refreshCurrentSection() -> loadSection()`。
- 表格 loader 会在请求前写入“加载中...”，然后整体替换 `tbody.innerHTML`。
- 总览三个请求完成后分批写入 UI，趋势图每次销毁并重建 Chart 实例。
- 用户自动刷新会清空批量选择。
- 阶段 2 已让后台请求失败保留已有栏目内容并显示过期提示。

应用内浏览器当前阻止访问本机回环服务，Chrome 也没有可用的 ChatGPT Chrome
Extension，因此 DevTools Network/Performance 和 60 秒录屏基线仍沿用阶段 0 记录，
无法完成的真实浏览器项会继续保持未勾选。

基线检查：

- `node --check enterprise-server/static/dashboard.js`：通过。
- 现有前端 Node 测试：13 个通过。
- Enterprise Server：170 个测试通过、0 个失败；仍有基线中的 62 条 warning。

## 2. 专项阶段 1：统一刷新上下文

已完成：

- 定义唯一的 `RefreshMode.INITIAL/MANUAL/AUTO` 和 `isSilentRefresh()`。
- `loadSection()` 接收模式、向 loader 传递 `{ mode, signal }` 并返回成功布尔值。
- 栏目切换使用 `INITIAL`；筛选、分页、按钮和 mutation 后刷新使用 `MANUAL`。
- 60 秒定时器显式使用 `AUTO`，未修改 `AUTO_REFRESH_MS`。
- 只有请求成功且用户仍停留在原栏目时，才更新最后成功时间。
- 自动刷新继续使用后台错误语义；取消请求不显示错误。
- 趋势、CLI 发布和文件中心的刷新按钮不再绕过统一刷新入口。

建议提交：`Add dashboard refresh modes`
