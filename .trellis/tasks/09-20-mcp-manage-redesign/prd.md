# MCP服务管理页面和功能重新设计

## Goal

重设计 MCP 服务管理页面与功能；按全局规则先出原型（`prototypes/`）再实现。

## Background（已确认事实）

- 现状入口：`src/pages/McpPage.tsx`（1333 行，经 `src/App.tsx` 的 `mcp` 页挂载），5 区块 + 1 弹窗：顶栏（标题 + 更新角标 + 检查更新/全部更新/应用到目标/手动添加）、Registry 卡（搜索 + 列表 + 分页）、扫描纳管卡（一键纳管/重扫/警告）、已保存 Table（名称/类型/目标/启用/操作）、须知卡；560 宽 Modal 表单（简单层 + 高级 Collapse）。
- 状态层：`src/stores/mcpStore.ts`（CRUD/仓库/扫描纳管 8 actions，映射 10 个后端 command），`src/stores/mcpUpdateStore.ts`（版本检查 TTL 30 分钟 + inFlight 去重）；类型 `src/types/mcp.ts`；测试 `src/pages/McpPage.test.tsx`（约 20 用例）+ `mcpUpdateStore.test.ts`；浏览器 mock `src/lib/browserMock.ts` 覆盖 mcp 全契约。
- 后端：`src-tauri/src/commands/mcp.rs`（10 command：list/get/save/delete/apply/paths/search/discover/scan/import）+ `mcp_update.rs`（3 command）；`adapters/mcp.rs`（四目标写盘：托管前缀 + 接管 + 原子 + 锁）、`mcp_scan.rs`（只读扫描，只回元数据）、`mcp_identity.rs`（coarse/equivalence 身份口径）、`mcp_update.rs`（npm 安装更新）、`mcp_registry/`（官方仓库）；`repo/mcp.rs`（`mcp_servers` 表 + 加密 + applied_targets）；i18n `mcp.` 前缀约 70 键（zh-CN/en-US）。
- 后端铁律（AGENTS.md「MCP 统一管控」，违反即破坏既有数据）：四目标原生位置；托管条目 `xiaobai_<name>`；只清理本前缀；备份 + 原子替换 + `.lock`；形状不合法报错保留；`env`/`headers` 加密存、明文落盘（UI 须声明）；扫描只回元数据、密钥后端重读；接管仅等价条目，不等价报错跳过；`mcp_servers` 留在 `FINGERPRINT_TABLES`（否则变更不触发同步）。
- 原型现状：`prototypes/` 与 `.trellis/prototypes/` 均无 MCP 原型，需新建。

## Key Decisions

- D1（已定）：前后端一起改；Rust 与数据兼容变更单独评估迁移方案。
- D2（已定）：四个方面全在必改范围 —— R1 页面布局交互；R2 应用目标流程；R3 扫描纳管规则；R4 Registry 安装体验。
- D3（已定）：分阶段交付 —— P0 原型全局一次覆盖 R1–R4 并评审确认；P1 实现 R1 + R2；P2 实现 R3 + R4；R5 数据兼容贯穿 P1/P2。
- D4（已定，P0 原型评审 2026-09-20，用户四项全按推荐项确认）：R1 三页签（我的 MCP / 发现安装 / 扫描纳管）；R2 应用面板页内常驻；R3 冲突条目默认不勾选、跳过该目标（不做强制覆盖，与后端接管铁律一致）；R4 免密钥条目无已有目标时弹窗问一次目标再装（不先入库不应用）。

## Requirements

- R1：重做 MCP 管理页面布局与核心交互。
- R2：简化应用目标流程（四目标勾选、改名/禁用/删除后清理、报错口径）。
- R3：调整扫描纳管规则（发现、等价接管、纳管写入）。
- R4：优化 Registry 安装体验（搜索、安装、密钥配置）。
- R5：数据兼容 —— 已有 `mcp_servers` 数据与各客户端已应用配置不受损。

## Acceptance Criteria

- P0：`prototypes/` 下 MCP 原型稿覆盖 R1–R4 信息架构与核心交互，用户评审确认。
- P1：新布局上线，应用目标流程可走通（勾选/清理/报错）；`pnpm typecheck` + 相关单测通过；存量数据与已应用配置无损（抽查四目标文件）。
- P2：新扫描纳管规则生效且旧数据可读；Registry 安装链路可用；`cargo test`（src-tauri）+ 前端相关测试通过；`mcp_servers` 同步指纹不受影响。

## Out of Scope

- 新增应用目标客户端（仍为 Claude Code / Codex / Pi / Prime 四目标）。
- MCP 协议本身与 server 端运行调试能力。
- WebDAV 同步/备份协议语义变更（仅要求不受本次影响）。

## Open Questions

- 无阻塞问题；P0 原型评审时可能产生新的交互细节决策，进 P1/P2 前补。
