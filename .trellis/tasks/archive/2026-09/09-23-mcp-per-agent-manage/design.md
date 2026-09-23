# Design — MCP 按 Agent 各自管理改版

## 现状（探查结论）

- 前端 `src/pages/McpPage.tsx` 是**单页平铺**：标题 + 工具条 + 一列 `McpCard`（每卡内嵌 4 个 target 开关）+ 扫描/纳管区 + 路径卡 + 弹窗。无侧栏。
- 应用中心 `src/pages/ApplyPage.tsx` 已是**目标侧栏壳**：`ApplySidebar`（`w-56`，`@lobehub/icons` 图标 + `StatusDot` + antd `Menu`）+ keep-alive 的 per-target 面板；tab 状态在 `uiStore.applyTab`。→ **直接照搬这套壳**。
- MCP store `src/stores/mcpStore.ts` 已有：list/get/save/delete/apply、`search_mcp_registry`/`discover_mcp_registry`、`scan_existing_mcp`/`import_scanned_mcp`。**仓库搜索（R5）后端+前端已存在**，本期只是把它放进新壳。
- 后端 apply 主干 `commands/mcp.rs::apply_to_targets`（备份+锁+原子写+`xiaobai_` 清理+未托管冲突 error-skip）已完备；per-client 写盘在 `adapters/mcp.rs::apply_to_{claude,codex,pi,prime}`，共用 `merge_servers_into_json`。
- **无任何 per-target 已应用指纹**；`mcp_applied_targets` 存在 `sync_meta`（local-only，`NOISY_SYNC_TABLES`，不进指纹）。

## 变更边界

### 净新增
1. 前端：MCP 页从单页 → 目标侧栏壳（Claude/Codex/Pi，无 Prime）。
2. 后端：`mcp_drift_status` 命令（干跑 diff，不写盘）→ 驱动「需重新应用」。
3. 绝对路径标记：`McpServerSummary` 派生布尔字段 + 卡片标。
4. 每 Agent「重新检测」按钮（复用 `scan_existing_mcp`，前端按 target 过滤）。

### 复用/不改
- 仓库搜索、扫描纳管、apply 主干、加密、identity/takeover 口径、`xiaobai_` 前缀、`FINGERPRINT_TABLES`。
- Prime 后端保持不动；仅前端 MCP 侧栏不列 Prime。

## 详细设计

### D1 前端侧栏壳
- `uiStore` 加 `mcpTab: McpAgentTab`（`"claude_code"|"codex"|"pi"`）+ `setMcpTab`，默认 `claude_code`（照抄 `applyTab` 实现）。
- 新 `src/components/mcp/McpSidebar.tsx`：照抄 `ApplySidebar`，`TAB_KEYS` 去掉 prime；`StatusDot` 的 active 表示「该 Agent 有已启用 MCP」；漂移时点变橙（用 `StatusDot` 变体或加一个橙点）。计数 = 该 target 已启用托管条目数。
- `McpPage.tsx` 重构为壳：左 `w-56` + `McpSidebar`，右 keep-alive per-target 面板 `McpAgentPanel`（新 `src/pages/mcp/McpAgentPanel.tsx`），props `target`。
- `McpAgentPanel` 内容（复用现有子组件）：
  - 顶部：`重新检测` + `添加 MCP` 按钮 + 漂移提示条（见 D2）。
  - ①已启用（托管）：`servers.filter(s => s.targets.includes(target))` → `McpCard`，卡上开关退化为**该 target 单开关**（关闭 = 从 `targets` 移除该 target 后 `saveServer`，触发 apply 清理）。编辑/删除保留。
  - ②未纳管：`scanOutcome.entries.filter(e => e.target===target && !e.managed)` → `UnmanagedMcpCard` + 纳管。
  - ③内置/插件说明：静态文案（不接管不同步）。
- `ConflictModal`、add/edit 表单弹窗（含 registry/manual `Segmented`）从旧页原样迁移；添加时 target 默认勾选当前 Agent。

### D2 漂移对账（无新持久状态）
- **不存指纹**：漂移 = **实时干跑 diff**（DB 期望的托管条目 vs 客户端文件现状）。更真实、无跨机同步隐患、免 `sync_meta` 改动。
- 重构 per-client 写盘：抽出纯函数 `plan_json_merge(current_map, servers, target) -> McpMergePlan { to_write: Vec<String>, to_clean: Vec<String>, conflicts: Vec<String> }`；`merge_servers_into_json` = `plan` + 执行写。Codex 侧同样抽 `plan_codex_merge`。
- 新命令 `mcp_drift_status(state) -> Vec<McpTargetDrift>`：对 Claude/Codex/Pi，读当前文件（复用 `mcp_scan::scan_target`/parse）→ 算 plan（不写）→ 回 `{ target, to_write: usize, to_clean: usize, conflicts: Vec<String>, drift: bool }`。`drift = to_write>0 || to_clean>0 || !conflicts.is_empty()`。
- 前端 `mcpStore.loadDrift()` → `Record<target, McpTargetDrift>`；侧栏橙点 = `drift`；面板顶部提示条显示「待写入 N · 待清理 M」+`重新应用`；重新应用 = `applyServers([target])` 后 `loadDrift()`+`loadServers()`+`runScan()`。
- 触发刷新 drift 的时机：进入页面、save/delete/toggle 后、同步拉取后（若有同步完成事件则监听，否则页面 focus 时刷新）。

### D3 绝对路径预警
- `mcp_identity.rs` 加 `pub fn command_is_absolute(kind, config) -> bool`：stdio 且 `command` 首段 `Path::is_absolute` 或匹配 `^[A-Za-z]:\\`。
- `repo::mcp::read`（或 list 组装处）用它派生 `absolute_command: bool` 塞进 `McpServerSummary`（**派生字段，非 DB 列，不进指纹**）。
- 前端 `McpServerSummary` 加 `absoluteCommand?: boolean`；`McpCard` 显示「跨机可能失效」Tag。表单保存时若命中给一次 `message.warning`。

### D4 每 Agent 重新检测
- 复用 `scan_existing_mcp`（全量扫描），面板按 `target` 过滤展示；按钮触发 `runScan()`。不写盘。

## 数据/兼容
- 无新 DB 列、无 `FINGERPRINT_TABLES` 改动、无新 `sync_meta` 键。`McpServerSummary.absolute_command` 与 `McpTargetDrift` 都是运行时派生，不落库。
- `xiaobai_` 前缀、identity/takeover、扫描不回密钥值：全部不动。

## 命令表面（新增）
- `mcp_drift_status() -> Vec<McpTargetDrift>`（注册进 `lib.rs` invoke_handler）。
- 现有 `apply_mcp_servers(targets)` 复用为「重新应用」。

## 测试计划
- Rust：`plan_json_merge`/`plan_codex_merge` 单测（to_write/to_clean/conflict 各情形）；`mcp_drift_status` 集成测（tempdir：无漂移=空、编辑后=to_write、删除后=to_clean 孤儿、未托管冲突=conflict）；`command_is_absolute` 单测；确认 `apply_servers_to_targets` 行为不变（既有 27+25 测通过）。
- 前端：`pnpm typecheck` + `pnpm test:run`；`browserMock` 补 `mcp_drift_status` mock，`McpServerSummary` mock 加 `absoluteCommand`。
- 手动：切 Agent 只显示该目标；编辑触发漂移条→重新应用消除；绝对路径卡显示标；registry 搜索在新壳内可用。
