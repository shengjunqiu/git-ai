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

## 3. 专项阶段 2：稳定表格刷新

已完成：

- 增加 `replaceHtmlIfChanged()`，先按元素上下文规范化 HTML，再比较并按需写入。
- 组织、开发者、项目、AI 工具、用户和 API Key 表格只在内容变化时替换。
- 自动刷新不调用表格 loading renderer；首次和手动刷新保留明确加载态。
- 分页请求失败不再清空 `nextCursor` 和 `hasMore`。
- 分页按钮内容不变时不重建。
- 部门请求先写入局部结果，成功后才替换树；保留有效父层级，父节点消失时回根。
- CLI 发布统计、发布表格和文件表格分别比较内容后更新。
- 用户自动刷新保留当前批量选择，只移除在当前页消失或变为已授权的选择。

首列和表格 DOM 不再因无变化的自动刷新重建，模态框和 mutation 按钮不属于刷新提交范围。

建议提交：`Keep dashboard tables stable during refresh`

## 4. 专项阶段 3：复用图表实例

已完成：

- 总览趋势图、趋势分析图和 Agent 对比图都只在没有实例时创建。
- 数据签名不变时不写入 Chart.js 数据，也不触发 `update()`。
- 自动刷新有数据变化时使用 `update('none')`；首次和手动刷新保留默认动画。
- 总览和 Agent 对比图在空数据时只切换 canvas/空状态，不销毁已有实例。
- 趋势图只在单点 `bar` 与多点 `line` 类型真正变化时重建。
- Agent 对比区域补充空状态，并支持有数据与无数据双向切换。

浏览器控制台实例对比和 Performance 面板检查保留到专项阶段 7。

建议提交：`Update dashboard charts without rebuilding`

## 5. 专项阶段 4：稳定总览更新

已完成：

- summary、Top 开发者和趋势请求继续并发，但统一通过 `Promise.allSettled()` 收集。
- 所有请求结束后再同步提交成功区域，避免按网络返回顺序分波写入 DOM。
- 单个接口失败时保留该区域旧内容，同时允许其他成功区域更新。
- 统计卡、标题、CLI 状态、详情、class 和 title 只在值变化时写入。
- Top 开发者先生成完整 HTML，再通过 `replaceHtmlIfChanged()` 按需替换。
- CLI 状态后台刷新失败时保留最近一次有效状态；首次或手动读取失败仍展示错误。

Performance 面板和连续两轮 60 秒无闪动检查保留到专项阶段 7。

建议提交：`Avoid unchanged dashboard DOM updates`

## 6. 专项阶段 5：防止重叠和过期覆盖

已完成：

- 使用按栏目 `Map` 管理进行中的请求，不再用单一全局 controller 取消所有栏目。
- 同栏目 AUTO 与进行中请求冲突时直接跳过，不叠加第二轮请求。
- 同栏目 MANUAL 与进行中请求冲突时排队一次；连续点击共享同一个排队 Promise。
- 新栏目 INITIAL 可立即开始；同栏目的新 INITIAL 会取消并替换旧请求。
- 旧栏目请求可完成隐藏区域更新，但不会修改当前栏目的刷新时间或 CLI 状态。
- 标签页隐藏时定时器跳过请求；恢复可见后触发一次 AUTO，并继续遵守防重入规则。

慢网速 Network 面板和快速切换的浏览器验收保留到专项阶段 7。

建议提交：`Prevent overlapping dashboard refreshes`

## 7. 专项阶段 6：自动化回归与完整检查

新增 `dashboard-refresh.test.cjs`，使用 Node 内置 `node:test` 验证：

- AUTO 不写入 loading，INITIAL/MANUAL 保留 loading。
- 相同 HTML 不替换，不同 HTML 仅替换一次。
- 同栏目冲突策略为 AUTO 跳过、MANUAL 排队、INITIAL 替换。
- 图表数据不变时实例保持空闲，AUTO 数据变化时调用 `update('none')`。

检查结果：

- `node --check enterprise-server/static/dashboard.js`：通过。
- `node --test enterprise-server/static/*.test.cjs`：17/17 通过，其中专项 4/4。
- `cargo test --manifest-path enterprise-server/Cargo.toml`：170/170 通过。
- `task lint`：被既有 MDM 告警阻断；`src/mdm/agents/codebuddy.rs`、`qoder.rs`、`trae.rs` 中共 4 个未使用导入或 `needless_return` 错误，与本次 Dashboard 改动无关。

建议提交：`Test silent dashboard refresh behavior`
