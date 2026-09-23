# Implement — MCP 按 Agent 各自管理改版

## 执行顺序（后端先，前端后，边做边测）

### 阶段 1 — 后端：plan 重构 + 漂移命令（不改行为）
1. `adapters/mcp.rs`：抽 `plan_json_merge(current, servers, target) -> McpMergePlan`；`merge_servers_into_json` 改为调用 plan 再写。Codex 抽 `plan_codex_merge`。跑既有 `cargo test`（27+25 测须全绿，证明行为不变）。
2. `adapters/mcp_identity.rs`：加 `command_is_absolute(kind, config) -> bool` + 单测。
3. `commands/mcp.rs`：加 `mcp_drift_status` 命令（读文件→plan→`Vec<McpTargetDrift>`，非 Prime）。`lib.rs` 注册。加集成测（tempdir：clean/edit/delete-orphan/conflict）。
4. `repo::mcp` / summary 组装：给 `McpServerSummary` 加派生 `absolute_command`。

### 阶段 2 — 前端：数据层
5. `src/types/mcp.ts`：加 `McpAgentTab`、`McpTargetDrift`、`McpServerSummary.absoluteCommand?`。
6. `src/stores/mcpStore.ts`：加 `drift`、`loadDrift()`（`invoke("mcp_drift_status")`）。
7. `src/lib/browserMock.ts`：加 `mcp_drift_status` mock；summary mock 补 `absoluteCommand`；确认 registry/scan mock 仍匹配新壳。
8. `src/stores/uiStore.ts`：加 `mcpTab` + `setMcpTab`（照抄 applyTab）。

### 阶段 3 — 前端：侧栏壳
9. 新 `src/components/mcp/McpSidebar.tsx`（照抄 `ApplySidebar`，去 Prime，加漂移橙点 + 已启用计数）。
10. 新 `src/pages/mcp/McpAgentPanel.tsx`（target 面板：重新检测/添加按钮 + 漂移条 + 已启用/未纳管/内置 三区）。
11. 重构 `src/pages/McpPage.tsx` 为壳（w-56 + McpSidebar + keep-alive 面板）；迁移弹窗（表单含 registry/manual、ConflictModal）；per-target 单开关逻辑。
12. `McpCard.tsx`：单 target 开关变体 + 绝对路径 Tag。

### 阶段 4 — i18n + 收尾
13. `zh-CN.json` / `en-US.json`：加 mcp 侧栏/漂移/路径/重新检测文案（复用已有 `mcp.*` / `apply.*` 键优先）。
14. 全量校验。

## 验证
- `cd src-tauri && cargo test`（重点：既有 mcp 测不回归 + 新 drift/abs 测通过）。
- `pnpm typecheck && pnpm test:run`。
- 手动过一遍 6 条验收。

## 明确不做
- 不拆 `mcp_servers` 表、不改 `targets_json` 模型。
- 不接聚合站（Smithery/mcp.so）。
- 不动 Prime 后端；不做 P1 同步后自动重应用（本期只做手动一键重应用）。
- 不改 identity/takeover/`xiaobai_`/`FINGERPRINT_TABLES`/加密协议。

## 风险
- plan 重构若改动写盘字节顺序 → 既有 27 测会红；以「测不变」为守门。
- Codex `toml_edit` 干跑需克隆 doc 避免副作用。
- 侧栏橙点若每次 render 都拉 drift 会抖动 → drift 只在明确时机刷新（进页/写后/focus）。
