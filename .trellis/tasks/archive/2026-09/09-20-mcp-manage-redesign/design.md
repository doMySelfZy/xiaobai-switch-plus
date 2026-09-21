# MCP 重设计技术方案（design.md）· 单列表方向

## 架构与边界

- **本轮以前端重写为主，后端 command 表面保持不变**（已核实现有 `list/get/save/delete/apply/paths/discover/search/scan/import` 足以支撑新原型全部交互）。高风险文件 `mcp_identity.rs`、`repo/mcp.rs`、`adapters/mcp.rs` **不动**。
- `src/pages/McpPage.tsx` 从「三页签容器」重构为「单列表页」：
  - 顶栏：搜索框 + 「添加 MCP」+ 检查更新（删客户端筛选条、删图例行）。
  - 主体：`McpList` = 已托管卡片段 + 分隔线 + 未纳管卡片段。
  - 卡片组件：复用/改造 `src/pages/mcp/SavedMcpList.tsx` → 单卡 `McpCard`（名称/类型/命令预览/总开关/编辑删除/更新提示 + 四客户端开关行 + 冲突横幅）。
  - 未纳管卡：新增 `UnmanagedMcpCard`（虚线框 + 「未纳管·在 X」+ 纳管按钮）。
  - 添加弹窗：`AddMcpModal`，内部两来源 tab（手动 / 仓库安装）；`ApplyPanel.tsx` 常驻应用面板取消（应用改为卡片即时触发）。
  - 冲突弹窗：新增 `ConflictModal`（左右对比 config + 密钥键名，两选按钮）。
- 类型以 `src/types/mcp.ts` 为准；新文案一律 `mcp.` 前缀中英双语（zh-CN/en-US 同步，零漂移）。

## 数据流与契约

- **逐客户端开关（D6）**：点开关 → 取该 server 当前 `targets`，增/删对应 `TargetKind` → `mcpStore.saveServer(input)` → `save_mcp_server` 自动 `apply_to_targets` 写盘 → 回显 sweep 结果。无需新 command。
- **自动扫描 + 内联（D7）**：mount 调 `scan_existing_mcp`（现状已调）；`ScanOutcome.entries` 按 `managed/importedId/nameConflict/adoptable` 分流：已托管→并入卡片状态；未纳管且非冲突→未纳管卡；就地/全部纳管→ `import_scanned_mcp`。
- **仓库安装（D9）**：进入即 `discover_mcp_registry`（推荐）；搜索走 `search_mcp_registry(term, cursor)`；免密钥直接 `save`（默认 targets = 现有已用目标或四目标），需密钥走必填表单。
- **冲突两选（D10）**：
  - 「该客户端跳过」= 不改动，维持现状默认行为（应用时后端本就跳过不等价条目）。
  - 「用客户端的（反向入库）」= `import_scanned_mcp` 该条 → 建/指向库内行，使两边一致。
- `browserMock.ts` 的 mcp 契约随任何 command 变更同步（本轮预期零 command 变更，仅前端调用组合变化，仍需校验 mock 覆盖新调用路径）。

## 兼容与迁移

- 零 schema 变更（不动 `repo/mcp.rs`）；`mcp_servers` 保留在 `FINGERPRINT_TABLES`，`FINGERPRINT_ALGORITHM_VERSION` 不变。
- 不改 `xiaobai_` 命名空间、备份/原子/锁、非法形状保留、扫描只回元数据等铁律。
- 存量已应用配置：上线前抽查四目标文件（托管条目在、用户条目未动）。

## 已知实现项（非产品决策）

- **「反向入库」撞重名校验**：冲突条目按定义是 `name_conflict`（同名不同身份），而 `import_scanned_mcp → repo::mcp::save` 对同名不同身份**会失败并保留已有行**（测试 `commands/mcp.rs:958`）。实现「反向入库」时需在前端/命令层定策略：
  - 方案 A（推荐，纯前端）：反向入库前提示用户「客户端里的这条将以新名字纳管（如 `<name>-codex`）」，改名后 `import`，避开重名校验，不动后端。
  - 方案 B（改后端）：新增「用客户端定义更新库内同身份行」的命令——但冲突恰是「不同身份」，等价行本就自动接管，故 B 收益有限且触及 repo 层，不优先。
  - 决策留到实现批次开始时定；默认走 A。

## 关键取舍

- 单列表整页重构 vs 继续渐进：本轮选**较大幅度前端重构**（三页签→单列表是结构性改变，渐进拼接反而更乱），但严格限制在前端 + i18n，后端零改动以压低回归面。
- 应用面板下沉到卡片即时开关：去掉「勾选→底部面板点应用」两步，符合 D6；`ApplyPanel` 组件退役。
- 冲突「用我的覆盖」放弃：守 AGENTS.md 红线，仅保留不破坏用户手改的两个出口。

## 运维与回滚

- 前端重构单批次可 `git revert`；后端零改动。
- 回滚点：实现前备份 `~/.xiaobai-switch` 数据目录；上线前备份四目标客户端 MCP 配置片段。

## 验证

- 前端：`pnpm typecheck`、`pnpm test:run`（`McpPage.test.tsx` 必跑，随新结构重写用例）。
- 后端：本轮预期不改；若最终走方案 B 才需 `src-tauri` 下 `cargo test` + 指纹复查。
