# Enterprise Dashboard 自动刷新闪烁优化任务清单

本文档把 Enterprise Dashboard 的自动刷新闪烁问题拆成可以逐步执行、逐步验证、逐步提交的工程任务。

当前 Dashboard 每 60 秒调用一次 `refreshCurrentSection()`。除帮助页外，当前激活栏目会重新执行完整加载流程。表格页面通常先把已有内容替换成“加载中...”，请求完成后再整体重建表格；图表页面会销毁已有 Chart.js 实例后重新创建，因此即使服务端数据没有变化，用户仍会看到表格闪白、内容跳动或图表重新播放动画。

本次优化的目标不是取消自动刷新，也不是简单延长刷新间隔，而是把自动刷新改成“后台获取、静默更新、无变化不重绘”。

## 执行原则

1. 每次只执行一个阶段，完成该阶段验证后再进入下一阶段。
2. 首次进入栏目、手动点击刷新和后台自动刷新必须使用明确的不同模式。
3. 后台刷新期间保留当前可用内容，不显示中间“加载中...”状态。
4. 新数据请求成功后才更新页面；后台请求失败时保留旧数据。
5. 数据或生成的 HTML 没有变化时，不写入 DOM。
6. Chart.js 实例应复用，自动刷新时禁止 `destroy()` 后重建。
7. 自动刷新不得重置分页页码、cursor、部门层级或当前筛选条件。
8. 同一栏目同一时间最多允许一个后台刷新请求，避免慢请求重叠。
9. 不改变现有 API、鉴权、分页协议和 60 秒默认刷新间隔。
10. 每个阶段都记录修改文件、测试命令和结果。

## 当前影响范围

自动刷新入口位于 `enterprise-server/static/dashboard.js`：

```text
startAutoRefresh()
  -> setInterval(..., AUTO_REFRESH_MS)
  -> refreshCurrentSection()
  -> loadSection(currentSection)
  -> 当前栏目的 load*()
```

### 明显闪烁：表格先清空再整体重建

| 页面 | 加载函数 | 当前行为 | 优化重点 |
| --- | --- | --- | --- |
| 组织 | `loadOrgs()` | `setTableLoading()` 后整体替换表格 | 自动刷新不清空表格 |
| 部门 | `loadDepartments()` | 清空表格，拉取全部部门，再重建当前层级 | 保留当前层级和旧表格 |
| 开发者 | `loadDevs()` | 清空表格后整体替换 | 保留分页状态和 Git 信息 |
| 项目 | `loadProjects()` | 清空表格后整体替换 | 保留当前页 |
| AI 工具 | `loadTools()` | 清空表格后整体替换 | 保留当前页 |
| 用户管理 | `loadUsers()` | 清空表格后整体替换 | 保留当前页和管理操作状态 |
| API 密钥 | `loadApiKeys()` | 清空表格后整体替换 | 保留当前页 |
| CLI 版本发布 | `loadReleaseManagement()` | 先写入加载行，再重建统计与表格 | 分别静默更新统计和表格 |
| 文件中心 | `loadManagedFiles()` | 先写入加载行，再重建表格 | 保留已有文件列表 |

### 轻度闪烁：内容或图表重绘

| 页面 | 加载函数 | 当前行为 | 优化重点 |
| --- | --- | --- | --- |
| 数据总览 | `loadOverview()`、`loadOverviewTrend()` | 统计卡片分批更新；Top 开发者整体替换；趋势图销毁重建 | 汇总更新、HTML 去重、图表原地更新 |
| 趋势分析 | `loadTrends()` | 趋势图和 Agent 对比图销毁重建 | 复用 Chart.js 实例 |

### 不受影响

| 页面 | 原因 |
| --- | --- |
| 安装与使用指南 | `loadSection('help')` 不请求数据，也不修改 DOM |

## 非目标

- 不调整服务端聚合 SQL、rollup 或分页实现。
- 不改变 `AUTO_REFRESH_MS = 60000` 的默认值。
- 不把轮询替换成 WebSocket、SSE 或其他推送协议。
- 不重新设计 Dashboard 的视觉样式。
- 不在本次任务中重构与自动刷新无关的创建、删除、上传和发布操作。
- 不改变首次加载和用户主动刷新时的错误反馈语义。

## 推荐刷新模型

统一使用刷新模式，而不是给每个页面增加彼此不一致的布尔参数。

```js
const RefreshMode = Object.freeze({
    INITIAL: 'initial',
    MANUAL: 'manual',
    AUTO: 'auto',
});

function isSilentRefresh(options) {
    return options?.mode === RefreshMode.AUTO;
}
```

所有栏目加载函数逐步统一为：

```js
async function loadDepartments({ mode = RefreshMode.INITIAL } = {}) {
    const silent = mode === RefreshMode.AUTO;
    if (!silent) {
        setTableLoading('departments-table', 6);
    }

    // 先请求数据，成功后再决定是否更新 DOM。
}
```

模式语义：

| 模式 | 使用场景 | 显示加载态 | 请求失败行为 | 允许加载动画 |
| --- | --- | --- | --- | --- |
| `INITIAL` | 首次进入或切换栏目 | 是 | 显示当前栏目错误态 | 是 |
| `MANUAL` | 用户点击刷新、改变筛选条件 | 是，可沿用现有体验 | 显示错误反馈 | 是 |
| `AUTO` | 60 秒定时刷新 | 否 | 保留旧内容，仅记录错误 | 否 |

## 阶段 0：记录基线

目标：在修改代码前确认闪烁范围，并留下可重复对比的基线。

### 0.1 确认工作区

步骤：

- [x] 查看当前工作区状态：

```bash
git status --short
```

- [x] 记录已有未提交改动，避免覆盖用户工作。
- [x] 确认本任务主要修改 `enterprise-server/static/dashboard.js`；只有增加测试时才修改其他文件。

验收标准：

- [x] 已记录工作区状态。
- [x] 已确认本任务与其他未提交改动没有冲突。

### 0.2 记录当前自动刷新行为

步骤：

- [ ] 启动 Enterprise Server 和其本地依赖。
- [ ] 登录 Dashboard。
- [ ] 打开浏览器开发者工具的 Network 面板，启用 Preserve log。
- [ ] 分别停留在总览、趋势、部门和任意一个管理表格页面超过 60 秒。
- [ ] 记录每个页面刷新时发出的请求、可见闪烁和当前筛选/分页是否保留。
- [ ] 使用 Performance 面板录制一次自动刷新，记录 Layout、Paint 和 Chart 动画情况。
- [ ] 在 Network 面板使用 Slow 3G 或自定义延迟重复测试，确认“加载中...”持续时间。

验收标准：

- [x] 确认只有当前激活栏目发起自动刷新请求。
- [x] 确认部门等表格页面在请求开始时清空已有内容。
- [x] 确认总览和趋势页面的图表会销毁重建。
- [x] 保存至少一组优化前截图或录屏，供阶段 7 对比（复用阶段 0 三视口截图）。

### 0.3 运行基线检查

步骤：

- [x] 检查 JavaScript 语法：

```bash
node --check enterprise-server/static/dashboard.js
```

- [x] 运行 Enterprise Server 测试：

```bash
cargo test --manifest-path enterprise-server/Cargo.toml
```

验收标准：

- [x] JavaScript 语法检查通过。
- [x] Enterprise Server 测试通过，或已记录与本任务无关的既有失败。

## 阶段 1：建立统一刷新上下文

目标：让刷新入口知道本次加载属于首次加载、手动刷新还是自动刷新，并让异步完成状态可以向上传递。

### 1.1 定义刷新模式

步骤：

- [x] 在自动刷新常量附近增加 `RefreshMode`。
- [x] 增加 `isSilentRefresh(options)` 辅助函数。
- [x] 不修改 `AUTO_REFRESH_MS`。

验收标准：

- [x] 刷新模式只在一个位置定义。
- [x] 后续 loader 不直接判断计时器或全局时间，而是读取显式模式。

### 1.2 让 `loadSection()` 返回 Promise

步骤：

- [x] 把 `loadSection(id)` 改为 `loadSection(id, options)`。
- [x] 每个 loader 使用 `return loadX(options)` 的等价映射并可等待。
- [x] `help` 返回 `Promise.resolve()` 或保持为可等待的空结果。
- [x] `activateDashboardSection()` 以 `INITIAL` 模式加载栏目。
- [x] 时间范围、趋势指标和粒度等用户操作以 `MANUAL` 模式刷新。

验收标准：

- [x] 调用者可以 `await loadSection(...)`。
- [x] 切换栏目和现有按钮功能不变。
- [x] URL、浏览器前进/后退和权限判断不受影响。

### 1.3 改造 `refreshCurrentSection()`

步骤：

- [x] 接收 `{ mode = RefreshMode.MANUAL }`。
- [x] 记录刷新开始时的 section ID。
- [x] `await loadSection(sectionId, { mode })`。
- [x] 只在请求成功且用户仍停留在同一栏目时更新“上次刷新时间”。
- [x] 自动刷新使用 `RefreshMode.AUTO`。
- [x] 刷新按钮和时间范围选择使用 `RefreshMode.MANUAL`。

推荐结构：

```js
async function refreshCurrentSection({ mode = RefreshMode.MANUAL } = {}) {
    const sectionId = currentSection;
    const refreshed = await loadSection(sectionId, { mode });
    if (refreshed !== false && currentSection === sectionId) {
        updateRefreshTime();
    }
}
```

验收标准：

- [x] “上次刷新时间”不再早于数据请求完成时间。
- [x] 自动刷新与手动刷新模式可以从调用链传到具体 loader。
- [x] 某个 loader 的异常不会形成未处理的 Promise rejection。

### 阶段 1 验证

```bash
node --check enterprise-server/static/dashboard.js
```

- [x] 语法检查通过。
- [ ] 首次进入、切换栏目、前进/后退和手动刷新均正常。
- [ ] 此阶段允许页面仍然闪烁；行为分流完成即可。

建议提交信息：

```text
Add dashboard refresh modes
```

## 阶段 2：消除表格页面的中间加载态

目标：自动刷新时保留现有表格，等请求成功后再一次性更新。

### 2.1 改造通用表格 loader

依次处理：

- [x] `loadOrgs(options)`
- [x] `loadDevs(options)`
- [x] `loadProjects(options)`
- [x] `loadTools(options)`
- [x] `loadUsers(options)`
- [x] `loadApiKeys(options)`

每个函数按以下顺序修改：

1. 接收 `options` 并计算 `silent`。
2. 仅在 `!silent` 时调用 `setTableLoading()`。
3. 请求期间保留现有 DOM。
4. 请求成功后生成新的表格 HTML。
5. 仅当新 HTML 与当前 `tbody.innerHTML` 不同时才替换。
6. 自动刷新失败时保留旧表格；首次或手动加载失败时保留现有错误态。
7. 不重置 `tablePageState`、页码、cursor 或分页按钮。

建议增加通用辅助函数：

```js
function replaceHtmlIfChanged(element, nextHtml) {
    if (!element || element.innerHTML === nextHtml) return false;
    element.innerHTML = nextHtml;
    return true;
}
```

验收标准：

- [x] 六个表格页面自动刷新时不出现“加载中...”。
- [x] 数据不变时不替换表格 DOM。
- [x] 当前页码和 cursor 保持不变。
- [x] 首次加载、空状态、错误状态和分页按钮仍正常。

### 2.2 单独改造部门页

步骤：

- [x] 把 `loadDepartments()` 改为接收刷新模式。
- [x] 自动刷新时不调用 `setTableLoading('departments-table', 6)`。
- [x] 先把请求结果保存在局部变量中，不要在请求完成前覆盖 `departmentTreeRows`。
- [x] 请求成功后再更新 `departmentTreeRows`。
- [x] 保留有效的 `activeDepartmentParentId`。
- [x] 只有当前父部门已不存在时才退回根层级。
- [x] 让 `renderDepartmentBreadcrumb()` 和 `renderDepartmentLevel()` 使用“内容变化才写入”的方式。
- [x] 自动刷新失败时保留旧部门树、面包屑和当前层级。

验收标准：

- [ ] 在根部门和任意子部门层级停留超过 60 秒均不闪烁（待浏览器验收）。
- [x] 自动刷新后仍处于原部门层级。
- [x] 当前层级数据变化时只更新一次。
- [x] 当前父部门被删除后安全回到根层级。
- [x] 非管理员的“我的部门”视图行为不变。

### 2.3 改造版本发布和文件中心

步骤：

- [x] `loadReleaseManagement(options)` 自动刷新时不写入中间加载行。
- [x] 发布统计卡和发布表格分别使用内容比较后更新。
- [x] `loadManagedFiles(options)` 自动刷新时不写入中间加载行。
- [x] 文件表格使用内容比较后更新。
- [x] 后台请求失败时保留已有发布信息和文件列表。

验收标准：

- [x] 版本发布和文件中心自动刷新时不清空内容。
- [x] 上传、发布、删除等用户操作完成后仍能主动刷新并显示结果。
- [x] 模态框和操作中的按钮状态不会被自动刷新打断。

### 阶段 2 验证

```bash
node --check enterprise-server/static/dashboard.js
```

- [ ] 所有表格页面首次进入时仍显示加载状态。
- [ ] 所有表格页面自动刷新时保留旧内容。
- [ ] 开启网络延迟后，旧数据会持续显示直到新数据成功返回。
- [ ] 模拟请求失败后，自动刷新不会把有效表格替换成错误行。

建议提交信息：

```text
Keep dashboard tables stable during refresh
```

## 阶段 3：复用图表实例

目标：总览和趋势页面自动刷新时原地更新 Chart.js 数据，不销毁 canvas 上的现有图表。

### 3.1 改造总览趋势图

步骤：

- [ ] 把创建 `overviewTrendChart` 的配置提取到单独函数。
- [ ] 没有实例时才创建 Chart。
- [ ] 已有实例时更新 labels 和 datasets。
- [ ] 自动刷新调用 `overviewTrendChart.update('none')`。
- [ ] 首次加载和用户主动改变时间范围时可保留正常动画。
- [ ] 数据从非空变为空时隐藏 canvas 并显示空状态。
- [ ] 数据从空变为非空时恢复 canvas，必要时创建实例。
- [ ] 仅在页面生命周期结束或确有必要时调用 `destroy()`。

验收标准：

- [ ] 总览自动刷新时 Chart 实例引用保持不变。
- [ ] 数据不变时图表没有动画或闪白。
- [ ] 空数据和恢复有数据的状态切换正确。

### 3.2 改造趋势分析图

步骤：

- [ ] 对 `trendChart` 应用相同的复用策略。
- [ ] 当图表类型需要在单点 `bar` 和多点 `line` 之间切换时，先确认 Chart.js 是否支持安全修改 `config.type`。
- [ ] 如果类型切换必须重建，只在类型确实变化时重建，不要每次刷新都重建。
- [ ] 自动刷新使用 `update('none')`。

验收标准：

- [ ] 数据点数量和图表类型不变时不重建实例。
- [ ] 单点与多点切换正确。
- [ ] 指标和粒度选择器行为不变。

### 3.3 改造 Agent 对比图

步骤：

- [ ] 复用 `agentComparisonChart`。
- [ ] 更新 labels 和 dataset 后调用合适的 `update()`。
- [ ] 自动刷新禁用动画。
- [ ] 处理比较数据从有到无、从无到有的切换。

验收标准：

- [ ] 趋势页面两个图表均不会在自动刷新时闪烁。
- [ ] 图例、坐标轴、颜色和横向柱状图布局保持不变。

### 阶段 3 验证

- [ ] 在浏览器控制台记录刷新前后的 Chart 实例，确认实例复用。
- [ ] 使用 Performance 面板确认自动刷新不再产生完整图表销毁与重建。
- [ ] 测试空数据、单点数据、多点数据和指标切换。

```bash
node --check enterprise-server/static/dashboard.js
```

建议提交信息：

```text
Update dashboard charts without rebuilding
```

## 阶段 4：稳定总览内容更新

目标：总览中的统计卡和 Top 开发者只在数据变化时更新，避免三个并发请求造成分阶段跳动。

### 4.1 让总览请求先完成再提交 UI

步骤：

- [ ] 保留 summary、developers 和 trend 的并发请求。
- [ ] 请求函数先返回数据，不要在各自请求完成时立即修改 DOM。
- [ ] 使用 `Promise.allSettled()` 收集结果。
- [ ] 对成功部分一次性提交 UI 更新。
- [ ] 自动刷新失败的部分保留原内容。
- [ ] 首次加载仍显示明确的空状态或错误状态。

验收标准：

- [ ] 总览统计卡不再按请求完成顺序分波跳动。
- [ ] 单个接口失败不清空其他成功区域。
- [ ] 请求仍保持并发，不把总耗时改成串行之和。

### 4.2 避免无变化 DOM 写入

步骤：

- [ ] 统计文本仅在值变化时写入 `textContent`。
- [ ] Top 开发者先生成 `nextHtml`，再调用 `replaceHtmlIfChanged()`。
- [ ] 不重新创建完全相同的进度条 DOM。
- [ ] `loadClientStatus()` 对状态、详情、class 和 title 使用变化检查。

验收标准：

- [ ] 数据完全不变时，总览自动刷新不产生可见变化。
- [ ] CLI 状态变化时仍能在下一次自动刷新中显示。

### 阶段 4 验证

- [ ] 使用 Performance 面板对比优化前后的 Recalculate Style、Layout 和 Paint。
- [ ] 连续观察两轮无数据变化的自动刷新，页面无闪动。
- [ ] 修改或注入一组数据后，变化能在下一轮刷新正确显示。

建议提交信息：

```text
Avoid unchanged dashboard DOM updates
```

## 阶段 5：防止刷新重叠和过期响应覆盖

目标：网络较慢、用户快速切换栏目或连续点击刷新时，不出现并发请求互相覆盖。

### 5.1 增加按栏目刷新状态

步骤：

- [ ] 使用 `Map<sectionId, Promise>` 或等价结构记录进行中的刷新。
- [ ] 同一栏目已有自动刷新进行中时，跳过新的自动刷新。
- [ ] 不使用会阻塞所有栏目的单一全局布尔锁。
- [ ] 用户切换到新栏目时允许新栏目的首次加载立即开始。
- [ ] 手动刷新与自动刷新冲突时，明确采用“复用当前请求”或“手动请求优先”的策略，并写入代码注释。

推荐第一版策略：

```text
同栏目 AUTO + in-flight：跳过
同栏目 MANUAL + in-flight：等待当前请求结束后再执行一次
不同栏目：允许并行，但过期栏目不得更新当前栏目的刷新时间
```

验收标准：

- [ ] 慢请求超过 60 秒时不会叠加新的同栏目自动请求。
- [ ] 快速切换栏目不会阻塞新栏目的首次加载。
- [ ] 旧栏目的响应不会修改新栏目的“上次刷新时间”。

### 5.2 处理页面可见性

步骤：

- [ ] 评估在 `document.hidden === true` 时是否跳过自动刷新。
- [ ] 推荐隐藏标签页时跳过请求，恢复可见后触发一次静默刷新。
- [ ] `visibilitychange` 监听器只注册一次。
- [ ] 恢复可见时仍遵守同栏目防重入规则。

验收标准：

- [ ] 页面长期处于后台时不持续发送无意义请求。
- [ ] 返回页面后数据会静默追平。
- [ ] 不会因为恢复可见和定时器同时触发而发送两轮请求。

### 阶段 5 验证

- [ ] 使用网络限速让请求持续超过一个刷新周期。
- [ ] 检查 Network 面板，同一栏目没有重叠自动请求。
- [ ] 快速切换总览、部门、趋势页面，确认不会显示过期数据。
- [ ] 切换浏览器标签页并等待超过 60 秒，再返回验证恢复刷新。

建议提交信息：

```text
Prevent overlapping dashboard refreshes
```

## 阶段 6：补充可回归验证

目标：让刷新模式和关键无闪烁约束能够被后续修改者重复验证。

### 6.1 增加最小自动化检查

当前仓库没有明确的 Dashboard 浏览器单元测试框架。第一版不应仅为本任务引入大型前端依赖。

步骤：

- [ ] 保留 `node --check` 作为必跑语法检查。
- [ ] 如果实现中提取了不依赖 DOM 的纯函数，使用 Node 内置 `node:test` 为刷新模式和内容比较逻辑增加测试。
- [ ] 如果不适合提取纯函数，在 PR 中明确记录浏览器手动验证矩阵，不写只检查字符串存在的脆弱测试。
- [ ] 服务端 API 未修改时，仍运行 Enterprise Server 完整测试防止静态资源或路由回归。

最低测试场景：

- [ ] `AUTO` 模式不调用 loading renderer。
- [ ] `INITIAL` 和 `MANUAL` 模式仍允许 loading renderer。
- [ ] 相同 HTML 不触发 DOM 替换。
- [ ] 不同 HTML 只触发一次替换。
- [ ] 同栏目重复自动刷新被合并或跳过。
- [ ] 图表数据不变时不重建 Chart 实例。

### 6.2 执行完整检查

```bash
node --check enterprise-server/static/dashboard.js
cargo test --manifest-path enterprise-server/Cargo.toml
task lint
```

如果增加 Node 测试，再运行对应命令，例如：

```bash
node --test enterprise-server/static/*.test.js
```

验收标准：

- [ ] JavaScript 语法检查通过。
- [ ] 新增的刷新逻辑测试通过。
- [ ] Enterprise Server 测试通过。
- [ ] 根仓库 lint 通过，或已记录与本任务无关的既有失败。

建议提交信息：

```text
Test silent dashboard refresh behavior
```

## 阶段 7：浏览器验收和发布确认

目标：从用户视角确认所有栏目在真实自动刷新中稳定，并完成优化前后对比。

### 7.1 页面验收矩阵

对每个页面至少执行一次首次加载、一次手动刷新和两次自动刷新：

| 页面 | 首次加载态 | 手动刷新 | 自动刷新无闪烁 | 状态保留 |
| --- | --- | --- | --- | --- |
| 数据总览 | [ ] | [ ] | [ ] | 时间范围 [ ] |
| 趋势分析 | [ ] | [ ] | [ ] | 指标/粒度 [ ] |
| 组织 | [ ] | [ ] | [ ] | 当前页 [ ] |
| 部门 | [ ] | [ ] | [ ] | 当前层级 [ ] |
| 开发者 | [ ] | [ ] | [ ] | 当前页 [ ] |
| 项目 | [ ] | [ ] | [ ] | 当前页 [ ] |
| AI 工具 | [ ] | [ ] | [ ] | 当前页 [ ] |
| 用户管理 | [ ] | [ ] | [ ] | 当前页 [ ] |
| API 密钥 | [ ] | [ ] | [ ] | 当前页 [ ] |
| CLI 版本发布 | [ ] | [ ] | [ ] | 操作状态 [ ] |
| 文件中心 | [ ] | [ ] | [ ] | 操作状态 [ ] |
| 安装与使用指南 | [ ] | 不适用 | [ ] | 不适用 |

### 7.2 异常场景验收

- [ ] 使用慢速网络时，自动刷新始终保留旧内容。
- [ ] 自动刷新接口返回错误时，页面不被错误行覆盖。
- [ ] 用户主动刷新失败时仍能看到明确错误反馈。
- [ ] 数据从有到无、从无到有时，空状态切换正确。
- [ ] 分页最后一页数据变化后，分页按钮状态正确。
- [ ] 部门父节点被删除后安全回到根层级。
- [ ] 图表从单点变多点、从多点变空时显示正确。
- [ ] 非管理员和管理员角色分别验证可访问栏目。

### 7.3 性能和网络验收

- [ ] 无数据变化的自动刷新不产生整表 DOM 替换。
- [ ] 无数据变化的自动刷新不销毁 Chart 实例。
- [ ] 同栏目不存在重叠自动请求。
- [ ] 后台标签页不持续产生轮询请求；恢复可见后只补一次刷新。
- [ ] 与阶段 0 的录屏对比，表格闪白、内容跳动和图表重播均已消失。

## 完成定义

只有满足以下全部条件，任务才算完成：

- [ ] 60 秒自动刷新仍然有效。
- [ ] 除真实数据变化外，所有动态栏目自动刷新时无可见闪烁。
- [ ] 自动刷新不显示中间 loading 状态。
- [ ] 自动刷新失败不会清空或覆盖已有有效内容。
- [ ] 图表自动刷新不销毁并重建实例。
- [ ] 数据不变时不重写主要表格和列表 DOM。
- [ ] 分页、筛选、部门层级和用户操作状态得到保留。
- [ ] 同栏目请求不会重叠，过期响应不会覆盖新状态。
- [ ] JavaScript 语法检查和 Enterprise Server 测试通过。
- [ ] 已完成所有页面的浏览器验收矩阵。
- [ ] PR 描述包含问题说明、实现方式、测试结果和优化前后对比。

## 回滚与兼容性

- 本次改造不涉及数据库迁移和 API 协议变化。
- 如果静默刷新出现严重回归，可以先让自动刷新回退到现有 `loadSection(currentSection)` 行为。
- 不建议通过删除自动刷新或单纯调大 `AUTO_REFRESH_MS` 作为长期回滚方案。
- 若 Chart.js 原地更新在特定类型切换中不稳定，只允许对“图表类型真实变化”的情况局部回退到重建实例。
- 回滚时不得移除首次加载、手动刷新、分页和错误状态的现有能力。
