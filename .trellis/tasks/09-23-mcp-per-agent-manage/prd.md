# MCP 按 Agent 各自管理改版

## Goal

MCP 页从「统一列表 + 每卡勾多客户端」改为与**应用中心同壳**的按 Agent 侧栏：左侧 Claude / Codex / Pi（排除 Prime），点某个 Agent 只看/只管它的 MCP。同时补齐三块能力：每 Agent 独立「重新检测」、跨机漂移对账（需重新应用）、绝对路径预警、从仓库搜索添加。

底层数据模型**沿用** `mcp_servers` 单表 + `targets_json`（一份定义、多目标），不做按 Agent 拆表；「按 Agent」只是同一批记录的按目标过滤视图。

## Scope（本期全部一起做）

1. 按 Agent 侧栏 UI 改版（复用应用中心 ApplySidebar 同款壳）
2. 每 Agent 独立「重新检测」入口（复用 `scan_existing_mcp`，按当前 target 过滤）
3. 漂移对账：每 target 记「已应用指纹」，DB 现状 ≠ 已应用 → 侧栏角标 + 顶部提示条 + 一键重新应用
4. 绝对路径预警：stdio 命令首段为绝对路径时打标（跨机可能失效）
5. 从仓库搜索添加：官方 MCP Registry 为主，搜索→选用→自动填命令与密钥键名模板→补 key 值保存

排除：Prime 目标（按 09-22-remove-prime）；聚合站（Smithery/mcp.so）本期不接。

## Requirements

### R1 按 Agent 侧栏
- 左侧 w-56 侧栏列 Claude Code / Codex / Pi，含图标、状态点、该 Agent 已启用 MCP 计数；视觉与 `ApplySidebar` 一致。
- 右侧内容分区：①该客户端已启用的 MCP（托管条目，带每 Agent 开关/编辑/删除）②该客户端已有、未纳管（虚线卡 + 纳管）③内置/插件 MCP 说明（不接管、不同步）。
- 开关 = 该记录 targets 是否含当前 Agent；关闭即从该客户端清理 `xiaobai_<name>`，不影响其它 Agent。

### R2 每 Agent 重新检测
- 每 Agent 页一个「重新检测」按钮，只读扫描该客户端原生配置，刷新未纳管列表；不写盘。

### R3 漂移对账（需重新应用）
- 每个 target 持久化「上次应用时的托管清单指纹」。
- 触发漂移的场景：本机编辑/新增/删除/改 targets、WebDAV 同步拉取导致 DB 变、远端删除留下的孤儿。
- 漂移时：侧栏该 Agent 亮橙点；内容区顶部提示条显示待写入/待清理孤儿数量 + 「重新应用」按钮；重新应用后指纹对齐、角标消失。
- 重新应用复用既有 apply 路径（备份 + 原子写 + 锁 + `xiaobai_` 前缀清理 + 未托管冲突 error-skip）。

### R4 绝对路径预警
- stdio 记录命令首段匹配 `^[A-Za-z]:\\` 或 `^/` 时，卡片打「跨机可能失效」标；添加/编辑保存时也提示。不自动改写。

### R5 从仓库搜索
- 添加 MCP 弹窗默认进「从仓库搜索」，可切「手动填写」；编辑走手动。
- 搜索走官方 MCP Registry API；结果列名称、来源、类型、命令、需要的密钥键名。
- 选用后自动填 name/kind/command 或 url、env/header 键名模板（值留空），切到手动页补 key 值后保存。
- Registry 请求失败/无网络时降级：提示并可切手动填写，不阻塞。

## 兼容红线（不得改动）
- 托管命名空间 `xiaobai_`（写入客户端的 `mcpServers` / `[mcp_servers.*]` 键）。
- `mcp_servers` 必须留在 `sync.rs` 的 `FINGERPRINT_TABLES`；新增的「已应用指纹」是每机本地状态，**不进** FINGERPRINT_TABLES（否则又会引起跨机指纹环）。
- 扫描只回元数据（`env_keys`/`header_keys`），密钥值不经前端；纳管即接管逻辑与等价判定不变。
- 每客户端只写自己的原生位置；既有形状不合法必须报错并原样保留文件。

## Acceptance Criteria
- [ ] MCP 页为按 Agent 侧栏（Claude/Codex/Pi），无 Prime；切 Agent 只显示该目标的 MCP。
- [ ] 每 Agent「重新检测」可刷新未纳管列表且不写盘。
- [ ] 修改/同步导致某 target 漂移时，侧栏角标 + 顶部提示条出现；「重新应用」后对齐且孤儿被清理。
- [ ] 删除某 MCP 后，重新应用会清掉客户端里对应 `xiaobai_<name>`（孤儿清理）。
- [ ] stdio 绝对路径命令在卡片显示预警标。
- [ ] 添加 MCP 可从官方 Registry 搜索→选用→补 key→保存并应用；Registry 不可用时可降级手动填写。
- [ ] `xiaobai_` 命名空间、`FINGERPRINT_TABLES`、扫描不回密钥值等红线不变。
- [ ] 前端 `pnpm test:run`、`pnpm typecheck` 通过；`src-tauri` `cargo test` 通过。

## Notes
- 数据模型沿用 `mcp_servers` 单表 + `targets_json`，不拆分。
- 原型：`prototypes/mcp-per-agent-manage.html`（已评审通过）。
