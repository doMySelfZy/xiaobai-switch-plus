# 技术设计：站点配置精简、请求头全局化与代理/MCP 修复

## 一条贯穿全文的硬事实（先读）

`compute_logical_fingerprint`（`src-tauri/src/sync.rs:72-117`）的实现是：对 `FINGERPRINT_TABLES` 里每张表执行 `SELECT * FROM <table> ORDER BY rowid`，然后**按列顺序逐个值**喂进 SHA-256。

推论（决定了本任务所有存储方案）：

| 改动 | 是否改变指纹 | 是否需要递增 `FINGERPRINT_ALGORITHM_VERSION` |
|---|---|---|
| 给 `settings` / `sites` / `mcp_servers` 等**已入清单的表加列** | **会**（每行多一个值） | **必须**，否则新旧版本对同一份数据算出不同指纹 → 恒定相反 → 反复互相覆盖（0.1.3/0.1.4 事故同款） |
| 新建一张**不在清单里**的表 | 不会 | 不需要 |
| 在现有 JSON 列里**新增一个键**（如 `settings.json`） | 不会改变列结构；值变化会正常触发同步 | 不需要 |

因此本设计的总原则是：**能不动指纹表结构就不动**；确需动时，在下面单列一节说明代价与取舍。

---

## R1 站点表单重排与协议自动化

### 现状

- `sites.protocol` 是 NOT NULL 且默认 `openai_compatible`（`domain/mod.rs:31-36` 的 `SiteProtocol::parse` 有兜底），前端 Select **没有空态**（`SiteFormModal.tsx:91,165,173`），但提示文案写着"不确定时可留空…自动检测"。
- `protocol` 被 7 处消费：模型拉取分支（`models_fetch/mod.rs:135`）、模型探测端点/请求体/鉴权（`model_probe/mod.rs:13-59`）、Pi/Prime 写 `api` 形态（`adapters/pi.rs:124-139`、`adapters/prime.rs:127-142`）、思考等级校验（`adapters/thinking.rs`、`domain/thinking.rs:193-238`）、ZCode 的 `api.type` 推断（`adapters/zcode.rs:79-84`）、本地代理路由（`local_proxy/routing.rs:68`）。
- **写盘路径是离线的，不能现场探测**——所以协议字段必须保留，"让用户不选"只能通过自动化实现，不能通过删字段实现。

### 设计（不加列、不改指纹）

不做持久化的"来源标记"，改用**"检测时机"**实现用户诉求：

1. **新建站点**：表单默认不预选具体协议，保存成功且用户未在本对话框内显式选择时，前端后台调用既有的 `test_site_connection`，把结果回填到该站点（复用 `SiteFormModal.tsx:210,214` 已有的回填逻辑，改为保存后触发）。
2. **已有站点**：协议就是库里的值，**不再自动重探**；用户想换协议有两个入口——点「测试连接」重新探测（结果覆盖），或直接在下拉里手动选。
3. UI：下拉 label 从"连接协议（可选）"改为"连接协议"，提示改为"默认自动检测；可手动指定"，删掉"可留空"的错误说法。

这样：用户侧表现为"不用管协议"（新建即自动），又不引入任何新列、不触碰指纹；手动选择天然持久（它就是库里的值）。

### 未采纳的方案

持久化 `protocol_source`（`auto` / `manual`）列更"精确"，但**会给 `sites` 加列 → 指纹变化 → 必须递增算法版本 → 对端未升级前同步暂停**。为一个纯 UI 语义付这个代价不划算，故不采纳。

### 布局重排（R1.2 / R1.3 / R1.5）

- 备注（`notes`）移到基本信息区（名称 / Base URL / API Key 之后）。同时删除 `shouldOpenAdvanced()` 里"备注非空就展开高级配置"的分支（`SiteFormModal.tsx:32-34`）。
- NewAPI 访问令牌 + 用户 ID 移出站点表单的高级配置。**落点选择**：它们服务的是额度探测（`quota_probe/mod.rs:1729-1742`），最自然的家是站点详情里的额度区域（`SiteQuotaRow.tsx` 已有"配置访问令牌"的提示位）。由于跨组件传值会增加耦合，备选是保留在站点表单但**单独成区并置于末尾**、文案明确"仅用于余额查询"。**倾向第一种**；若实现时发现站点详情面板承载不了表单，则退回第二种，并在 PR 说明原因。
- 高级配置内部分组重排：只留「Codex 私有能力」（本来就是独立折叠块）与「本站覆盖请求头」（见 R2），去掉现在混在一起的"站点信息 / 余额查询 / 请求头"三分区。

---

## R2 请求头：全局默认 + 站点覆盖

### 现状

- 站点级 `proxy_headers` 加密存于 `sites.proxy_headers_encrypted`（`repo/site.rs:60-87`），**唯一消费方**是本地代理转发（`local_proxy/forward.rs:171` 的 `merge_headers`，由 `server.rs:362` 调用）——直连路径完全不注入。
- 合并语义：调用方传入的列表按名覆盖客户端原有头（`forward.rs:146-178`），受保护头（host/content-length 等）禁改（`headers.rs:12-23`）。
- 占位符 `${API_KEY}` / `${SESSION}` / `${UUID}` 在转发时按"该请求绑定的站点"解析（`headers.rs:108-114`）——**全局化后这套语义不变**，因为请求解析仍会确定站点。

### 设计：新增 `local_proxy_headers` 表并纳入跨设备同步

**决策（按用户要求修正）：全局请求头必须跨设备同步。**

| 方案 | 跨设备同步 | 代价 |
|---|---|---|
| **A. 新建表 `local_proxy_headers` + 加入 `FINGERPRINT_TABLES` + `FINGERPRINT_ALGORITHM_VERSION` 1→2**（采纳） | 是 | 两端都必须升级；未升级时同步**显式暂停**并提示"请先升级对端"。发版说明必须写明 |
| B. 塞进 `settings.json` 新键（表结构不变，无需递增版本） | 是 | **旧版本一保存设置就丢这个键**——`save_settings` 每次都 `serde_json::to_string(AppSettings)`，结构体里没有的键被静默丢弃（`repo/settings.rs:47-55`）→ 混版本期间静默丢配置 |
| C. 新表不加入指纹清单 | 否 | 每台机器各配一次（原方案，已废弃） |

选 A：它是唯一"既同步、又不静默丢数据"的方案——把混版本风险交给引擎自带的版本守卫，表现为**可见的暂停 + 明确提示**，而不是悄悄丢配置。C 被否决是因为用户明确要求同步。

存储细节：

- 新表字段：`id`（固定 1）、`headers_encrypted`、`header_count`、`updated_at`。
- 建表走 `CREATE TABLE IF NOT EXISTS` 并挂进 `ensure_incremental_schema`（每个"库已存在"分支都要跑到）。
- 读写复用 `repo/site.rs:60-87` 的加解密与 `validate_proxy_headers` 校验口径（保存层与转发层各一次），**不放松加密**。
- **加入 `sync.rs:41-51` 的 `FINGERPRINT_TABLES`**（9 → 10 张表），并同步递增 `FINGERPRINT_ALGORITHM_VERSION` 1 → 2；`sync.rs:711-729` 那条"把版本号钉死在表清单上"的护栏测试必须一起更新，否则测试会拦下改动（这是设计意图，不要绕过）。
- 注意 `LEGACY_FINGERPRINT_ALGORITHM_VERSION` 保持 1 不变：0.1.4/0.1.5 写出的 manifest（无 `fingerprintAlgorithm` 字段）仍按 1 处理 → 与本机 2 不匹配 → 同步暂停并提示升级对端。这正是期望行为。

### 合并契约

`merge_headers(client, global, site_overrides)`：

1. 客户端原有头全透传（丢 hop-by-hop / `host` / `content-length`）；
2. 应用全局列表（同名覆盖）；
3. 应用站点覆盖列表（同名覆盖，**站点胜**）。

也就是说：**站点未配 = 用全局值；站点配了同名头 = 站点值覆盖全局值**。

需要改动的点：

- `local_proxy/server.rs:55-82` 的 `DatabaseResolver::resolve()` 多读一份全局头，装进 `ResolvedSite`；
- `local_proxy/forward.rs:146-178` 的 `merge_headers` 签名扩展为三段合并，`server.rs:362` 调用点同步；
- 前端：`ProxyPage.tsx` 新增「默认请求头」卡片，直接复用 `ProxyHeaderEditor` + `parseProxyHeadersJson`；`SiteFormModal.tsx` 里的编辑器文案改为「本站覆盖（可选）」；
- i18n 双份 + `browserMock.ts` 补 case。

---

## R3 本地代理页精简与端口 bug

### 现状（bug 成因）

`commands/proxy.rs:83-115` 改端口的顺序是：

1. 先把新端口落库；
2. 调 `stop()` —— 而 `stop()` 内部会 `disengage_takeover(state, false)`：**清空 `local_proxy_targets`、置 `enabled=false`，并把被接管目标写回直连**；
3. 再按**步骤 1 之前抓取的旧接管列表**逐个 `sync_applied_target`，把客户端重新指向 `http://127.0.0.1:<新端口>/<token>/t/<target>`。

结果：磁盘上 CLI 指向代理，而 DB/UI 显示"未接管"；此后退出应用时 `disengage_takeover` 遍历的是**空集合**，不做恢复 → CLI 永久指向一个没人监听的端口。

### 设计：把"运行意图"与"接管集合"解耦

- `local_proxy_targets`（settings JSON）= **用户希望接管的目标**，不因停止而清空。
- `local_proxy_enabled`（settings JSON）= 是否运行。
- `stop()`：置 `enabled=false` + 把所有 `targets` 写回直连（安全，避免指向死端口），**但保留 `targets` 列表**。
- `start()`：按 `targets` 重新接管。
- 改端口：落库 → 若在运行则重启监听（走"保留 targets 的重启"路径，**不经过会清空的旧 stop**）→ 按 `targets` 重新写客户端配置。三步后 DB、磁盘、UI 一致。

这条设计正好启用代码里已经写好但**从未被调用**的 `keep_enabled=true` 分支（`local_proxy/mod.rs:82-87` 的注释描述了它，两个调用点都传 `false`）。

### 界面精简

- **删掉标题栏的「启动代理 / 停止代理」按钮**，只保留「代理服务」卡片里的开关（它同时承担状态显示与切换）。两者当前绑定同一状态、调用同一函数（`ProxyPage.tsx:212-214` vs `221-227`），100% 重复。
- 「刷新」按钮：4 秒轮询已覆盖，改为轮询期间的禁用态 + tooltip 说明，或直接删除（实现时二选一并说明）。
- `portHint` 改为与实际一致（"修改后会自动重启监听并重写接管地址"）。
- 新增一行说明："停止代理会把所有已接管目标写回直连；重新启动会按上次的接管选择恢复。"
- 端口保存失败时重新拉取状态（`ProxyPage.tsx:136-138` 的 catch 目前不回滚，4 秒后输入框会跳到新值）。

---

## R4 MCP 检查更新

### 现状

- 解析器 `extract_package_name`（`commands/mcp_update.rs:129-155`）只认"命令里内联包名"或 `url` 以 `npm:` 开头；而 App 的两条安装路径（官方仓库安装 `mcp_registry/mod.rs:335-375`、手动新建）与扫描纳管（`adapters/mcp_scan.rs`）产出的**全是** `command: "npx"` + 包名在 `args` → 永远解析失败 → 静默无输出。
- 版本结果写在 `mcp_servers.current_version / latest_version / last_update_check_at`（`repo/mcp_version.rs:7-21`），而 `mcp_servers` **在 `FINGERPRINT_TABLES` 内且逐列哈希** → 每次检查都会改动本地指纹，可能把一次本该"远端变了→下载"的干净同步推成 `conflict=true`。
- 仓库里另有一份**未被调用**的实现 `adapters/mcp_version.rs`（307 行），其解析口径恰好支持 `command == "npx"` + args，还支持 uvx/pip —— 可直接移植其解析函数。

### 设计

1. **解析**：移植 `mcp_version.rs` 的 args 感知解析到 `mcp_update.rs`（覆盖 `npx`/`uvx`），保留现有函数作为兜底（不删，避免行为回退）。
2. **状态存储**：新建表 `mcp_update_state`（`server_id` 主键、`current_version`、`latest_version`、`checked_at`），**不加入 `FINGERPRINT_TABLES`**。"哪个 MCP 有新版本"是设备本地事实，且"检查"这个动作本身不该被当成数据变更——若入清单，每次检查都会弄脏指纹、可能把干净的同步推成 conflict（这正是要修的毛病）。
3. `mcp_servers` 上的三个版本列**保留但不写**（红线：不删列）。旧版本仍会写它们——那是既有行为，不改变算法，无兼容问题。
4. **UI**：解析不到的条目显示"不支持检查更新"（新增 i18n 键），不再静默；可解析条目维持现有更新/一键更新。
5. 顺带：`check_mcp_updates` 的 npm 子进程加超时（无超时会永久挂住按钮），并把每次进页面自动检查改为带 TTL 的按需触发——**注意这与 perf 任务的"McpPage 挂载时 5 次 invoke"条目重叠**，实现前先确认对方进度，避免重复改。

---

## R5 三个既有缺陷

1. **`detect_protocol` 重复请求**：尝试链里 `(Bearer, true)` 出现了两次（`models_fetch/mod.rs:223-230` 与 `:239-243` 的数组内），第二次必然失败（前一次已失败）→ 纯浪费一次 15s 超时的机会。删掉重复项即保持行为不变。
   **与 perf 任务冲突**：对方要改的正是这个函数的超时与并行结构（任务描述第⑤条）。本项**最后做**，动手前 `git log -p src-tauri/src/models_fetch/mod.rs` 确认对方已落地再改。
2. **WebDAV 远端剪枝误删指针包**：`webdav.rs:293-306` 按"本设备最新的 N 份"剪枝（默认 3），但 manifest 当前引用的那份**没有豁免**。修法：剪枝前读远端 manifest，把 `bundleFileName` 加入豁免名单（找不到 manifest 时按现状执行）。
3. **`sync.rs:139-141` 注释**：写着"缺省……按当前算法处理"，与实现（`LEGACY_FINGERPRINT_ALGORITHM_VERSION`）和规格文档都矛盾。只改注释，不动逻辑。
   **同文件时序**：perf 任务要改 `sync.rs` 的锁范围，改前确认无冲突。

---

## 兼容性与回滚

- **不删列**（全案遵守）。
- **指纹算法版本 1 → 2**（因 `local_proxy_headers` 进入清单）：升级后与未升级的对端**同步暂停**，界面提示"请先升级对端"，两端都升级后自动恢复。这是设计意图而非缺陷；**发版说明必须写明这条**。
- `mcp_update_state` **不进清单**，所以本任务只引入一次算法变更，不会因 MCP 检查再触发一次。
- 数据库迁移只用 `CREATE TABLE IF NOT EXISTS` + `ensure_incremental_schema`，可重复执行。
- 回滚：回退代码后算法版本回到 1；只要两端都回退即可继续同步。新表被忽略、全局请求头失效（回到"只有站点级"），站点级数据从未移动或删除 → 无数据损失。
- 打包验证：`cargo test`（src-tauri）、`pnpm test:run`、`pnpm typecheck`、最后 `pnpm tauri build --bundles nsis` 出一份安装包真机点一遍（尤其是"新建站点自动检测"和"改端口后重启应用"两条）。

## 明确不做

- 不实现跨设备同步的全局请求头（见 R2 取舍）。
- 不动技能中心 Agents 文案、ZCode 文案与真机验证（本次只回答"是什么"，不在本任务范围）。
- 不做性能 / 流畅度优化（归 `09-15-perf-smoothness`）。
