# Enterprise 前端阶段 3：移动端和响应式体验

记录日期：2026-07-20

对应计划：[`enterprise-frontend-optimization-task-plan.md`](./enterprise-frontend-optimization-task-plan.md)

## 1. 实施范围

阶段 3 保持现有 Dashboard 栏目、权限和 URL 协议不变，修复窄屏导航、宽表格、小屏表单
和帮助页代码块的可用性：

- `enterprise-server/static/dashboard.html` 增加移动端顶栏、抽屉控制和统一表格滚动区域。
- `enterprise-server/static/dashboard.js` 管理抽屉开关、焦点、Escape、Tab 循环和断点变化。
- `enterprise-server/static/dashboard.css` 定义 768px/480px 响应式布局、触控尺寸和
  reduced-motion。
- `enterprise-server/static/dashboard-responsive.test.cjs` 固化结构和行为约束。

## 2. 移动端导航

768px 及以下使用左侧抽屉：

- 顶栏始终显示菜单按钮、当前用户名和退出入口。
- 抽屉关闭时使用 `inert` 和 `aria-hidden` 阻止隐藏导航进入 Tab 顺序。
- 打开后焦点进入当前栏目；Tab/Shift+Tab 在抽屉内循环。
- 点击栏目、关闭按钮、遮罩或按 Escape 会关闭抽屉。
- 关闭后焦点回到打开菜单的按钮。
- 抽屉打开期间主内容不可交互、不可滚动。
- 普通开发者仍沿用既有 `.admin-only` 和 section 访问检查，不能看到或进入管理员栏目。
- 断点从移动端切换回桌面时会清理遮罩、`inert` 和抽屉状态。

## 3. 表格滚动

Dashboard 的 9 张表格现在都位于独立 `.table-scroll` 区域：

| 表格 | 最小内容宽度 |
| --- | ---: |
| 组织、项目、工具、部门 | 720px |
| 开发者、用户、API Key、CLI 发布、文件中心 | 980px |

每个滚动区域都有 `tabindex="0"`、`role="region"`、可读 `aria-label` 和可见焦点轮廓。
主内容禁止横向溢出，横向滚动只发生在对应表格内部；`.table-card` 不再通过
`overflow: hidden` 裁剪操作列。

本阶段评估后没有启用 sticky 首列：用户表的选择列、开发者复合身份单元格和多操作按钮在
窄屏下会形成不同的遮挡边界，统一 sticky 反而会压缩可读区域。后续如按表格分别设计，
应补遮挡和键盘滚动测试后再启用。

## 4. 小屏布局

- 768px 下页面头、Toolbar、表格工具条和发布表单允许换行。
- 480px 下统计卡变为单列，图表和卡片缩小内边距。
- 按钮、选择框、输入框、文件选择按钮、导航和帮助操作在移动端至少 44px 高。
- 模态框使用动态 viewport 高度和可滚动遮罩，降低软键盘遮挡风险。
- 480px 下帮助页复制按钮进入正常文档流，不再覆盖代码。
- `prefers-reduced-motion: reduce` 会关闭不必要的动画和过渡。
- 200% 页面缩放会进入与 768px 窄屏相同的重排路径，但仍需真实浏览器完成最终冒烟。

## 5. 自动化验证

```bash
node --check enterprise-server/static/dashboard.js
node --test \
  enterprise-server/static/dashboard-api.test.cjs \
  enterprise-server/static/auth-page-api.test.cjs \
  enterprise-server/static/dashboard-responsive.test.cjs
cargo test --manifest-path enterprise-server/Cargo.toml
```

响应式测试断言：

- 移动菜单、用户信息、退出、抽屉关闭和遮罩控制存在。
- 焦点恢复、Escape、Tab 循环、遮罩关闭、`inert` 和断点同步逻辑存在。
- 9 张表格与 9 个键盘可访问的滚动区域一一对应。
- 移动断点不再用 `display: none` 隐藏侧边栏。
- 单列统计卡、44px 触控尺寸、模态框高度、帮助复制按钮和 reduced-motion 规则存在。

本轮结果：JavaScript 语法检查通过；前端请求与响应式测试共 13 个全部通过；Enterprise
Server 170 个测试全部通过、0 个失败。编译仍输出已有的 62 条后端 warning。

## 6. 浏览器验证限制

本轮应用内浏览器阻止访问 `127.0.0.1`，Chrome 中也没有可用的 ChatGPT Chrome
Extension，因此没有虚报 390×844、768×1024、1024×768、1440×900 或跨操作系统的
真实浏览器结果。Chrome 响应式模式、Safari/WebKit/iOS 和 Windows 表格冒烟仍保留为
待补验收项；仓库内自动化测试用于防止结构回归，但不替代视觉和触控验收。

## 7. 后续边界

- 阶段 4 处理自动刷新时的选择、焦点、图表和 DOM 稳定性。
- 阶段 6 移除 inline handler 时，把移动导航监听也迁入模块化事件入口。
- 阶段 8 为模态框补完整 `role="dialog"`、标题关联、Escape 和焦点恢复协议。
