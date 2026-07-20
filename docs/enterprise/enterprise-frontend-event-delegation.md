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
