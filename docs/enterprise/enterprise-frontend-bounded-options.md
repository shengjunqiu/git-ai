# Enterprise Dashboard 阶段 5.2：有界表单选项执行记录

## 范围

本次改造覆盖以下四个管理端表单选择器：

- 创建用户的组织。
- 创建用户的部门。
- 创建部门的所属组织。
- 创建部门的上级部门。

原实现通过 `fetchAllPaginated()` 连续请求最多 50 页、每页 100 条。超过 5,000 条时，
选择器会无提示缺失后续数据，同时表单打开成本随组织或部门总数增长。

## API 语义

以下现有接口新增可选的 `q` 参数：

- `GET /api/admin/organizations/list`
- `GET /api/admin/departments`

组织按名称和 slug 做不区分大小写的包含匹配；部门按名称、编码和 slug 匹配。过滤发生在
游标比较和 `LIMIT` 之前，原有个人组织过滤、`org_id` 部门范围、父部门信息和成员统计保持
不变。

组织游标现在绑定 `include_personal` 和 `q`，部门游标绑定 `org_id` 和 `q`。把一个筛选条件
生成的游标用于其他筛选条件会返回 `400 Bad Request`，避免漏项或重复项。

## 前端行为

- 每次组织或部门请求最多返回 100 个选项，不再循环拉取所有分页。
- 无关键词的组织首屏结果在两个创建表单之间复用。
- 四个选择器都提供关键词搜索；输入后等待 250 毫秒发起请求。
- 新搜索会取消同一选择器尚未完成的请求，关闭弹窗会取消全部选项请求和防抖任务。
- 服务端返回 `has_more=true` 时明确显示“仅显示前 100 个”，并提示继续细化关键词。
- 搜索分别显示正在加载、无匹配结果、加载失败和成功数量。
- 创建部门的父部门没有匹配结果时仍可选择“无（根部门）”；加载失败时选择器保持禁用。

## 验证

已执行：

```bash
cargo fmt --manifest-path enterprise-server/Cargo.toml
node --check enterprise-server/static/dashboard.js
node --test enterprise-server/static/*.test.cjs
DATABASE_URL=postgresql://gitai:gitai@localhost:5433/gitai_enterprise \
  cargo test --manifest-path enterprise-server/Cargo.toml \
  organizations_and_departments_cursor_paginate_by_name -- --nocapture
DATABASE_URL=postgresql://gitai:gitai@localhost:5433/gitai_enterprise \
  cargo test --manifest-path enterprise-server/Cargo.toml
```

结果：

- 静态前端行为测试 20/20 通过。
- 组织/部门关键词、组织范围和筛选游标专项测试实际连接 PostgreSQL 后通过。
- Enterprise 完整测试 170/170 通过。
- `git diff --check` 通过。

浏览器端完整创建流程仍需要可登录的 Enterprise 管理员测试环境；该阻塞条件沿用阶段 4
浏览器验收记录。
