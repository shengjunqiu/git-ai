# Enterprise Dashboard 阶段 5.1：部门按层加载执行记录

## 范围

本次改造聚焦部门统计页的有界层级加载，不包含创建用户、创建部门表单的关键词搜索。

## API 语义

`GET /api/v1/aggregate/departments` 现在支持：

- 不传 `parent_id`：管理员返回根部门；部门受限用户仍返回自己的授权部门。
- 传 `parent_id=<uuid>`：管理员只返回该父部门的直属子部门。
- 返回 `has_children`、`parent_id`、`depth` 和包含全部后代的汇总指标。
- 返回 `parent_exists`，父节点已删除、跨组织或不可访问时统一返回 `false` 和空列表。
- 部门游标包含 `parent_id`；跨父节点复用游标返回 `400 Bad Request`。

权限过滤仍同时应用请求中的组织 slug 和登录身份的组织范围。部门受限用户的
`parent_id` 参数会被忽略，不能借此访问兄弟部门或其他子树。

## 前端行为

- 首次进入部门页只请求根层当前页，不再通过 `fetchAllPaginated()` 拉取整棵树。
- 点击有子部门的行时，携带 `parent_id` 请求下一层。
- 当前层使用统一 cursor 分页控件，每页 25 条。
- 已加载节点保存在本地，用于构造面包屑和返回上一级。
- 层级、页码和 cursor 组成缓存键，成功结果缓存 30 秒；返回已访问层级时可立即显示缓存，
  同时继续发起正常刷新。
- 服务端报告父节点不存在时，前端安全回到根层并重新请求。

## 验证

已执行：

```bash
cargo fmt --manifest-path enterprise-server/Cargo.toml
node --check enterprise-server/static/dashboard.js
DATABASE_URL=postgresql://gitai:gitai@localhost:5433/gitai_enterprise \
  cargo test --manifest-path enterprise-server/Cargo.toml department_aggregates
DATABASE_URL=postgresql://gitai:gitai@localhost:5433/gitai_enterprise \
  cargo test --manifest-path enterprise-server/Cargo.toml
```

部门聚合测试覆盖根层、直属子层、后代汇总、父节点不存在、跨层游标拒绝，以及受限用户
忽略 `parent_id`。测试已连接仓库 Docker Compose 中的 PostgreSQL 并实际执行，专项 3/3、
完整 Enterprise 170/170 通过；静态前端测试 17/17 通过。

尚待：

- 100、1,000、10,000 个部门的实际请求数和响应时间记录。
- 完整服务环境下的面包屑、分页和父节点删除浏览器验收。
- 创建部门上级选择器的关键词搜索和有界加载。
