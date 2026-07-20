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
手动规模基准 1/1、完整 Enterprise 174 项通过且 1 项基准按预期忽略；静态前端测试
28/28 通过。

### 规模基准

新增默认忽略的 `department_aggregate_scale_benchmark`，在独立临时数据库中为每个规模创建
一个组织和同一根层的部门，使用 PostgreSQL 16、启用 rollup 路径、每页 25 条调用真实
`aggregate_departments` handler。每组记录第一次调用，并在随后 7 次调用中计算中位数和
P95；测试结束后自动删除临时数据库。

运行命令：

```bash
DATABASE_URL=postgresql://gitai:gitai@localhost:5433/gitai_enterprise \
  cargo test --manifest-path enterprise-server/Cargo.toml \
  department_aggregate_scale_benchmark -- --ignored --nocapture
```

2026-07-20 本地 Docker 基准结果：

| 部门数 | 旧前端首屏请求数 | 新前端首屏请求数 | 返回行数 | 首次响应 | 预热中位数 | 预热 P95 |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 100 | 1 | 1 | 25 | 26.73 ms | 6.72 ms | 18.57 ms |
| 1,000 | 10 | 1 | 25 | 13.28 ms | 12.17 ms | 13.40 ms |
| 10,000 | 50，且只得到前 5,000 条 | 1 | 25 | 894.21 ms | 155.97 ms | 246.45 ms |

首屏 API 请求数和响应体行数已保持有界，不再随部门总数线性增长，也不再有 5,000 条
静默截断。基准同时暴露了一个后续服务端优化点：当前递归 CTE 仍会遍历组织的完整部门树，
因此 10,000 条数据下 SQL 计算时间仍随总量增长。本阶段解决的是前端全量拉取和请求瀑布；
后续应将“先选择当前层页面、再递归汇总选中节点的后代”作为独立数据库查询优化。

尚待浏览器验收：

- 完整服务环境下的面包屑、分页和父节点删除浏览器验收。
- 创建部门上级选择器的关键词搜索和有界加载。
