# git-ai 开发者端到端使用流程

本文档面向普通开发者和负责落地的管理员，说明开发者从第一次启用 `git-ai` 到日常提交、查看统计、上报后台 dashboard 的完整流程。

当前实现里，开发者身份、组织关系和 dashboard 数据隔离都在 enterprise server 中维护；CLI 本身不会自动注册新用户，也不会自动把开发者加入组织。

## 一、核心结论

开发者能否在管理端被正确识别并上传 Git 追踪信息，取决于四件事：

1. `users` 表里有该开发者用户。
2. `org_members` 表里有该开发者和公司组织的绑定关系。
3. 组织管理员已在管理端为该开发者开启“Git 追踪上传”授权。
4. 开发者本机的 `git-ai` 使用该开发者自己的凭证上传 metrics/report。

推荐的生产流程是：

```text
管理员创建开发者用户
-> 管理员把开发者加入公司组织
-> 管理员授权该开发者上传 Git 追踪信息
-> 管理员生成 install nonce
-> 开发者安装 git-ai 并兑换 nonce
-> 开发者正常使用 AI 工具和 git
-> git-ai 记录 AI 归因并在 commit 后上报 metrics
-> 管理端 dashboard 按用户和组织展示数据
```

不推荐让开发者直接依赖当前 `/verify` 设备码登录流程作为生产注册流程。当前 `/verify` 是临时实现，会把设备码授权给数据库里的第一个用户，不会让开发者选择自己的账号。

## 二、角色分工

| 角色 | 负责事项 |
| --- | --- |
| 系统管理员 | 部署 enterprise server、初始化首个 owner、维护组织、创建管理员 API key |
| 组织管理员 | 创建开发者用户、绑定组织和部门、逐人授权 Git 追踪上传、生成 install nonce、查看 dashboard |
| 开发者 | 安装客户端、兑换 nonce 登录、正常使用 AI 编码和 git、必要时手动补传数据 |
| CI/CD | 可选，用 API key 在流水线中上传 report 或补充历史统计 |

## 三、管理员准备工作

### 1. 确认服务端地址

示例：

```text
https://git-ai.example.com
```

确认服务可访问：

```bash
curl https://git-ai.example.com/health
```

### 2. 初始化首个管理员

首次部署时需要先有一个 owner 用户。当前 admin API 本身需要管理员权限，所以第一次建议直接用 SQL 初始化，示例见 [enterprise-server-deployment.md](./enterprise-server-deployment.md)。

初始化后，管理员可以通过 Bearer token 或带 `admin` scope 的 API key 访问 admin API。

### 3. 创建开发者用户

如果已有管理员 token：

```bash
ADMIN_TOKEN="<管理员 Bearer token>"
SERVER="https://git-ai.example.com"

curl -s -X POST "$SERVER/api/admin/users" \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  -H "Content-Type: application/json" \
  --data '{
    "email": "developer@example.com",
    "name": "Developer Name",
    "org_id": "<organization-uuid>",
    "department_id": "<department-uuid>",
    "generate_nonce": true
  }'
```

返回里会包含：

```json
{
  "id": "...",
  "email": "developer@example.com",
  "personal_org_id": null,
  "default_org_id": "<organization-uuid>",
  "department_id": "<department-uuid>",
  "install_nonce": "..."
}
```

`POST /api/admin/users` 不再创建个人组织；它会把用户加入指定组织和部门，并把该组织设置为默认组织。

新建的组织成员默认**未授权上传**。管理员必须在 dashboard 的“用户管理”中点击“授权上传”，或调用：

```bash
curl -s -X PUT "$SERVER/api/admin/users/<developer-user-id>/git-tracking-upload" \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  -H "Content-Type: application/json" \
  --data '{"authorized": true}'
```

授权只对管理员当前组织生效。同一开发者加入多个组织时，每个组织都要单独授权。撤销时提交 `{"authorized": false}`，已有 token 和 API key 会立即失去该组织的上传权限。

### 4. 调整组织角色

创建用户时默认角色是 `member`。如果要把用户设置为组织管理员，可以更新 `org_members` 表：

```bash
docker compose exec -T postgres psql -U gitai -d gitai_enterprise <<'SQL'
UPDATE org_members om
SET role = 'admin'
FROM users u
WHERE u.email = 'developer@example.com'
  AND om.user_id = u.id;
SQL
```

角色含义：

| 角色 | 含义 |
| --- | --- |
| `owner` | 组织所有者，管理员权限 |
| `admin` | 组织管理员，可看组织内全部数据 |
| `member` | 普通开发者，只能看自己的数据 |

### 5. 生成或发放 install nonce

如果创建用户时没有生成 nonce，可以单独生成：

```bash
curl -s -X POST "$SERVER/api/admin/install-nonces" \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  -H "Content-Type: application/json" \
  --data '{
    "user_id": "<developer-user-id>"
  }'
```

把返回的 `nonce` 和服务端地址发给开发者。

`install nonce` 是一次性登录兑换码。它不是最终 API key，也不是长期密码；开发者用它兑换自己的 `access_token` 和 `refresh_token` 后，nonce 会被标记为已使用。

## 四、开发者第一次安装和登录

开发者应从管理员处拿到：

| 信息 | 示例 |
| --- | --- |
| 服务端地址 | `https://git-ai.example.com` |
| install nonce | `8b1c...` |
| 安装包或安装脚本 | `install.sh` / `install.ps1` / 平台压缩包 |

### 1. Linux / macOS 安装

如果使用项目安装脚本，并且管理员给了 nonce：

```bash
INSTALL_NONCE="<管理员给你的 nonce>" \
API_BASE="https://git-ai.example.com" \
bash install.sh
```

安装脚本会做几件事：

1. 安装 `git-ai` 到本机。
2. 配置 `git` wrapper，让普通 `git` 命令经过 `git-ai`。
3. 调用 `git-ai exchange-nonce` 兑换登录凭证。
4. 调用 `git-ai install-hooks` 安装已支持 AI 工具的 hooks。
5. 把凭据绑定到 `API_BASE`，并自动写入 `~/.git-ai/config.json`。

如果是从 release 包安装，先按 [developer-install-guide.md](./developer-install-guide.md) 完成安装，再手动执行：

```bash
INSTALL_NONCE="<管理员给你的 nonce>" \
API_BASE="https://git-ai.example.com" \
git-ai exchange-nonce

git-ai install-hooks
```

### 2. Windows PowerShell 安装

示例：

```powershell
$env:INSTALL_NONCE = "<管理员给你的 nonce>"
$env:API_BASE = "https://git-ai.example.com"
.\install.ps1
```

安装完成后重新打开 PowerShell，让 PATH 生效。

如果需要手动兑换 nonce：

```powershell
$env:INSTALL_NONCE = "<管理员给你的 nonce>"
$env:API_BASE = "https://git-ai.example.com"
git-ai exchange-nonce
git-ai install-hooks
```

### 3. 验证登录身份

```bash
git-ai whoami
```

重点看：

```text
API Base URL: https://git-ai.example.com
Auth state: logged in
Email: developer@example.com
Organizations:
  - linewell.com (...) role=member
```

如果没有看到公司组织，说明管理员还没有把该用户写入 `org_members`，或者开发者本机拿到的不是自己的 token。

### 4. 验证 git wrapper 是否生效

Linux / macOS：

```bash
which git
git --version
git-ai --version
```

Windows：

```powershell
where git
git --version
git-ai --version
```

`git` 应优先指向 `~/.git-ai/bin/git` 或 `%USERPROFILE%\.git-ai\bin\git.exe`。如果没有经过 wrapper，commit 时可能不会生成完整的 authorship note。

### 5. 重启 IDE 和 AI agent

安装 hooks 后，建议重启正在运行的 IDE 和 AI agent，例如 Cursor、VS Code、Claude Code、Codex、Windsurf 等。部分 agent 启动时才读取 hook 配置。

## 五、开发者在项目里的日常流程

### 1. 进入项目

```bash
cd ~/Documents/Code/YourProject
git status
git-ai status
```

通常不需要在每个仓库执行初始化命令。安装后的 git wrapper 和 agent hooks 会在所有允许的仓库中工作。

如果 `git-ai status` 提示没有记录到 hooks，可以重新安装：

```bash
git-ai install-hooks
```

### 2. 正常使用 AI 工具写代码

开发者照常使用支持的 AI 工具编辑代码。支持的 agent/hook 会在编辑前后向 `git-ai` 写 checkpoint。

需要注意：

- `git-ai` 不是通过代码内容猜测 AI 代码。
- 只有安装并生效的 agent hook 上报过的编辑，才会被准确标记为 AI。
- 在启用 `git-ai` 之前已经存在的历史代码，不会自动被追溯成 AI。
- 如果某个提交上已经有 `refs/notes/ai`，`git-ai stats` 会读取这个已有 note。

### 3. 查看未提交归因

提交前可以看当前工作区的 AI / human 归因：

```bash
git-ai status
git-ai status --json
```

如果刚刚用 AI 改了代码，但这里完全没有记录，优先检查：

```bash
git-ai install-hooks
git-ai whoami
```

并重启 IDE / agent。

### 4. 正常提交代码

不需要改变 git 习惯：

```bash
git add .
git commit -m "Implement feature"
```

commit 成功后，`git-ai` 会把工作区 checkpoint 计算成 authorship log，并写到 Git Notes：

```text
refs/notes/ai
```

这个 note 记录了该 commit 中哪些行是 AI、human、mixed 或 unknown。

### 5. 本地检查 commit 归因

查看当前提交统计：

```bash
git-ai stats HEAD --json
```

查看当前提交的行级 AI 归属：

```bash
git-ai show HEAD
```

查看文件级 blame：

```bash
git-ai blame path/to/file.ts
```

如果需要查某段 AI 代码背后的 prompt：

```bash
git-ai search --file path/to/file.ts --lines 10-40 --verbose
```

## 六、数据如何出现在管理端 dashboard

管理端 dashboard 主要看两类数据：

1. 客户端自动或手动上传的 metrics，例如 commit 后的 AI 行数、工具模型统计。
2. `git-ai report upload` 上传的历史 report 数据。

### 1. commit 后自动上传 metrics

当开发者已登录，并且 `api_base_url` 指向企业服务端时，commit 后产生的 metrics 会尝试自动上传。

如果管理员尚未为当前开发者授权，服务端返回 403，CLI 会在本次 `git commit` 结束时明确显示：

```text
[git-ai] AI tracking saved locally. Upload blocked: an organization administrator must authorize Git tracking uploads for this developer account.
```

此时 Git commit 本身仍然成功，追踪事件只保存在本地，不会写入平台。管理员授权后，后续 commit 会自动重试本地队列。

如果当时网络不可用或 daemon 没有及时上传，事件会先进入本地队列：

```text
~/.git-ai/internal/metrics-db
```

开发者可以手动补传：

```bash
git-ai flush-metrics-db
```

如果 prompt/CAS 内容也需要补传：

```bash
git-ai flush-cas
```

### 2. 手动上传 report

如果需要把某个仓库或某段历史提交上传到服务端：

```bash
git-ai report upload . \
  --range HEAD^..HEAD \
  --server https://git-ai.example.com
```

上传全量历史时可以省略 range，但大仓库会更慢：

```bash
git-ai report upload . \
  --server https://git-ai.example.com
```

如果使用的是旧版轻量 report server，而不是 enterprise-server，也可能会用：

```bash
git-ai report summary . \
  --server https://git-ai.example.com
```

需要区分：`git push` 默认不会把 metrics/report 写入 enterprise-server。push 只是推 Git 对象；dashboard 要看到数据，仍然依赖 metrics 上传或 report upload。

### 3. 打开 dashboard

```bash
git-ai personal-dashboard
```

或者浏览器访问：

```text
https://git-ai.example.com/me
```

如果浏览器跳到登录页，需要输入：

- `gai_...` API key；或
- `eyJ...` 开头的 Bearer access token。

开发者本机 access token 可从凭证文件中取出：

```bash
jq -r '.access_token' ~/.git-ai/internal/credentials
```

Bearer token 默认 1 小时有效。长期浏览器登录更适合使用管理员创建的 API key，但普通开发者一般不需要 admin scope。

## 七、完整示例

假设：

```text
SERVER=https://git-ai.example.com
ORG_SLUG=linewell.com
DEVELOPER_EMAIL=developer@example.com
```

### 管理员侧

```bash
ADMIN_TOKEN="<管理员 Bearer token>"

curl -s -X POST "$SERVER/api/admin/users" \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  -H "Content-Type: application/json" \
  --data '{
    "email": "developer@example.com",
    "name": "Developer Name",
    "org_id": "<organization-uuid>",
    "department_id": "<department-uuid>",
    "generate_nonce": true
  }'
```

接口会把用户加入指定组织和部门；把返回的 `install_nonce` 给开发者。

在把 nonce 交给开发者前，管理员还需要在“用户管理”中为该用户点击“授权上传”，或者调用：

```bash
curl -s -X PUT "$SERVER/api/admin/users/<developer-user-id>/git-tracking-upload" \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  -H "Content-Type: application/json" \
  --data '{"authorized": true}'
```

### 开发者侧

```bash
INSTALL_NONCE="<管理员给你的 nonce>" \
API_BASE="https://git-ai.example.com" \
bash install.sh
```

重新打开终端后验证：

```bash
git-ai whoami
git-ai status
```

日常开发：

```bash
cd ~/Documents/Code/YourProject
# 使用 AI 工具编辑代码
git-ai status
git add .
git commit -m "Implement feature"
git-ai stats HEAD --json
git-ai show HEAD
git-ai flush-metrics-db
```

然后访问：

```text
https://git-ai.example.com/me
```

## 八、常见问题

### 1. 开发者执行 `git-ai login` 后，管理端会自动出现新用户吗？

不会。当前 `git-ai login` 只会给已有用户签发 token，不会注册新用户，也不会把用户加入组织。

而且当前 `/verify` 临时实现会授权给数据库里的第一个用户，不适合作为生产登录方式。生产建议用“管理员创建用户 + 绑定组织 + install nonce”的流程。

### 2. 为什么登录后 `whoami` 不是我的邮箱？

通常是因为使用了当前临时 `/verify` 设备码流程，服务端把你绑定到了数据库第一个用户。解决方式是让管理员为你的用户生成 install nonce，然后重新登录：

```bash
git-ai logout

INSTALL_NONCE="<你的 nonce>" \
API_BASE="https://git-ai.example.com" \
git-ai exchange-nonce

git-ai whoami
```

### 3. 为什么我提交了代码，但 dashboard 看不到？

按顺序排查：

```bash
git-ai whoami
git-ai config api_base_url
git-ai stats HEAD --json
git notes --ref=refs/notes/ai show HEAD
git-ai flush-metrics-db
```

如果本地 commit 有 note，但 dashboard 没有数据，通常是 metrics 没上传、用户没有绑定组织，或者浏览器登录的是另一个身份。

### 4. 为什么项目刚启用就显示已有 AI 行？

`git-ai` 不会扫描代码内容来猜 AI。出现已有 AI 行，一般是因为当前 commit 上已经有 `refs/notes/ai`，或者本地有历史 working log/checkpoint。可以检查：

```bash
git notes --ref=refs/notes/ai show HEAD
git-ai show HEAD
```

### 5. 以前没有启用 `git-ai` 的 AI 代码能追踪吗？

不能自动追踪。`git-ai` 只能准确追踪启用后由支持的 agent hook 上报的编辑，或者读取已经存在的 Git AI authorship note。

### 6. 开发者需要 API key 吗？

普通开发者通常不需要 API key。开发者日常使用推荐 Bearer token + refresh token，也就是通过 install nonce 兑换出的凭证。

API key 更适合：

- CI/CD 上传 report。
- 管理员后台操作。
- 长期机器身份调用 API。

如果 API key 带 `admin` scope，它就具备管理员权限，应严格保管。
