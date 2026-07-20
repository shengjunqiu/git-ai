# Enterprise Dashboard 安全启动数据

## 目标

阶段 6.2 删除 Dashboard 模板中的可执行内联变量，并保证角色界面在 JavaScript
启动前就处于正确的可见状态。

## 实现

- 服务端把 `isAdmin` 序列化到
  `<script type="application/json" id="dashboard-bootstrap">`。
- 通用序列化边界转义 `<`、`>`、`&`、U+2028 和 U+2029，避免数据中的
  `</script>` 提前结束脚本元素。
- `dashboard.js` 从元素的 `textContent` 读取 JSON；数据缺失、格式错误或类型不匹配时
  默认按非管理员处理。
- 服务端同时为 `body` 渲染 `dashboard-role-admin` 或
  `dashboard-role-member`。CSS 在首帧隐藏不属于当前角色的内容，不再等待 JavaScript
  逐项修改内联样式。
- 组织入口、系统管理入口、管理员统计卡及组织页面统一使用 `admin-only` 标记；
  普通成员专属的 git-ai 状态卡使用 `member-only`。

## 验证

```bash
node --test enterprise-server/static/*.test.cjs
cargo test --manifest-path enterprise-server/Cargo.toml
```

专项测试覆盖安全 JSON 转义、管理员与成员模板输出、bootstrap 解析失败时的降权行为，
以及所有角色专属节点的首帧隐藏标记。
