# Enterprise Dashboard 阶段 6.1：事件委托执行记录

## 初始盘点

2026-07-20 对 `dashboard.html` 和 `dashboard.js` 中的 `on*=` 属性做静态统计：

| 来源 | `onclick` | `onchange` | 合计 |
| --- | ---: | ---: | ---: |
| 静态 HTML | 53 | 8 | 61 |
| JavaScript 动态模板 | 36 | 1 | 37 |
| 总计 | 89 | 9 | 98 |

动态模板主要集中在分页、开发者和用户操作、部门层级、模态框、API Key、CLI 发布以及
文件管理。静态 HTML 主要集中在导航、筛选/刷新、管理入口和帮助页复制按钮。

## 第一批：导航、刷新和分页

本批建立统一委托入口，并先迁移低风险、调用频率高的控制：

- 在 `document` 上分别注册一个 `click` 和一个 `change` 监听器。
- 使用 `event.target.closest('[data-action]')` 支持按钮内部图标等嵌套点击目标。
- 导航通过 `data-section` 传递栏目，不再依赖 `showSection()` 的全局调用。
- 表格分页通过 `data-table-key` 和 `data-page-direction` 传递参数。
- 总览/趋势筛选、手动刷新、开发者排序和文件选择改用声明式 action。
- 静态管理入口也接入相同分发器，为后续迁移动态管理操作保留统一边界。
- 业务逻辑函数保持不变，事件层只负责读取参数和分发。

本批移除 33 个 inline handler；剩余 65 个集中在动态业务操作、模态框和帮助页复制按钮，
按阶段计划继续分批迁移。

## 验证

```bash
node --check enterprise-server/static/dashboard.js
node --test enterprise-server/static/*.test.cjs
```

新增 `dashboard-actions.test.cjs`，覆盖：

- 静态导航和动态分页不再生成 inline handler。
- 嵌套点击目标可以定位 action 元素。
- 栏目、分页方向和复选状态从 dataset 正确传入既有业务函数。
- `click` 和 `change` 委托监听器只注册一次。

当前静态前端测试 32/32 通过；完整 Enterprise 测试 174 项通过，1 项手动规模基准按预期
忽略。

## 第二批：动态管理操作

本批继续迁移通过表格重绘生成的管理操作：

- 用户选择、单个 Git 追踪上传授权/撤销、创建用户密钥和删除用户。
- API Key 撤销。
- CLI 版本提升为 latest 和复制安装链接。
- 文件固定链接复制、版本发布/删除和设置入口。

用户名称、密钥名称、版本、checksum、文件 slug、名称和说明分别写入语义明确的
`data-*` 属性；所有动态属性值使用 `escapeAttribute()`，事件分发时再由浏览器 dataset
还原。标识符和展示名称保持分离，API 路径中的标识符仍由原业务函数使用
`encodeURIComponent()` 编码。

本批没有改变确认弹窗、按钮禁用/恢复、上传 `activeUploads` 去重、文件校验、超时和错误
提示逻辑。新增测试使用包含单双引号、`&` 和尖括号的名称与说明，验证事件参数不会破坏
HTML 属性且传入业务函数后内容不丢失。

本批再移除 12 个 inline handler，累计移除 45 个，剩余 53 个仅位于开发者/部门入口、
动态模态框和帮助页复制按钮。当前静态前端测试 34/34 通过。
