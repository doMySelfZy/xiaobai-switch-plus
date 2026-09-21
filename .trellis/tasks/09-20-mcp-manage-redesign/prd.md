# MCP服务管理页面和功能重新设计

## Goal

把 MCP 管理页从「按功能分区（三页签：我的 / 发现 / 扫描）」重做成「**一张统一清单**」：用户在一个页面里管理四个客户端（Claude Code / Codex / Pi / Prime）的全部 MCP，卡片上逐客户端即时开关，安装/纳管随手可及，同名冲突当场可解。先出原型（已完成，见下）再实现。

## Background（已确认事实）

- **上一轮已上线但用户全面不满意**：`d205b47` 落地了「三页签 + 卡片列表 + 常驻应用面板」，用户对信息架构、应用流程、添加繁琐、发现/扫描难用四项全部不满意（2026-09-21 会话）。根因：页面按「功能分类」组织，而非用户实际动线（日常管理已有 + 随手安装新的）。
- **现状入口**：`src/pages/McpPage.tsx`（约 1460 行）+ `src/pages/mcp/`（`SavedMcpList.tsx`、`ApplyPanel.tsx`、`targets.ts`）。三页签（mine/registry/scan）+ 手动添加 Modal（560 宽，简单层 + 高级 Collapse）。
- **状态层**：`src/stores/mcpStore.ts`（CRUD/仓库/扫描纳管，映射后端 10 command）、`mcpUpdateStore.ts`（版本检查 TTL）；类型 `src/types/mcp.ts`；测试 `McpPage.test.tsx`（约 36 用例）；浏览器 mock `src/lib/browserMock.ts` 覆盖 mcp 全契约。
- **后端 command 表面**（`src-tauri/src/commands/mcp.rs`，已核实）：`list / get / save / delete / apply / paths / discover / search / scan / import`。
  - `save_mcp_server`：保存后自动 `apply_to_targets` 重新同步（改名/改目标/禁用会清理旧客户端里的托管条目）。每条 MCP 带 `targets: Vec<TargetKind>` = 应用到哪些客户端。
  - `scan_existing_mcp`：mount 时已调用；返回 `ScanOutcome`，每条带 `managed / importedId / nameConflict / adoptable` 标记。
  - `import_scanned_mcp` → `import_entry` → `repo::mcp::save`：纳管建行；**同名不同身份会撞重名校验而失败**（测试 `import_name_collision_fails_without_touching_existing`，`commands/mcp.rs:958`）。
  - 仓库来源：官方 MCP Registry `https://registry.modelcontextprotocol.io`（`src-tauri/src/mcp_registry/mod.rs`）。`search_url` 支持关键词 + cursor 翻页。**注册表不提供下载量/热度字段**，「首屏发现」用 6 个常见类目词（`DISCOVERY_TERMS`）合并去重，非真实热度榜。
- **后端铁律（AGENTS.md「MCP 统一管控」，违反即破坏既有数据）**：四目标原生位置；托管条目 `xiaobai_<name>`；只清理本前缀；备份 + 原子替换 + `.lock`；形状不合法报错保留；`env`/`headers` 加密存、明文落盘（UI 须声明）；扫描只回元数据、密钥后端重读；接管仅等价条目，**不等价报错跳过、绝不用库里旧版本覆盖用户改动**；`mcp_servers` 留在 `FINGERPRINT_TABLES`。
- **新原型（本轮设计事实来源）**：`prototypes/mcp-unified-manage.html`（已提交 `ecf07c1`，并按评审多轮迭代：删图例、删客户端筛选条、自动扫描内联、仓库搜索接真实交互、冲突弹窗改两选）。取代上一轮的 `prototypes/mcp-manage-redesign.html`（P0 评审稿，diff 说明性质）。

## Key Decisions

- D1–D3（沿用）：前后端一起评估；四方面（布局 / 应用 / 扫描 / Registry）全在范围；分阶段交付、数据兼容贯穿。
- **D5（本轮）单列表取代三页签**：主页面就是一张 MCP 卡片清单，无 tab。顶部只留搜索 + 「添加 MCP」+ 检查更新。删掉客户端筛选条与图例行（卡片自带客户端信息）。
- **D6（本轮）卡片保留逐客户端四开关，点击即时生效**：Claude/Codex/Pi/Prime 四开关在卡片上，点亮=已应用到该客户端。实现 = 改这条 MCP 的 `targets` 数组 → `save_mcp_server`（现有能力，无需新 command）。另有一个总开关（enabled）控制启用/停用。
- **D7（本轮）打开自动扫描、未纳管内联**：页面 mount 自动 `scan_existing_mcp`；用户直接在客户端手配、本工具未纳管的「野生」条目，作为卡片显示在主清单下半段（分隔线 + 「未纳管·在 X」标记 + 就地「纳管」），并支持「全部纳管」。删掉独立的「扫描纳管」入口。
- **D8（本轮）添加入口收拢为两来源**：「添加 MCP」弹窗只保留「手动填写」与「从仓库安装」两个来源；扫描纳管因自动化而不再是独立来源。手动表单把 env/headers/其他 JSON 收进「高级配置」折叠。
- **D9（本轮）仓库安装打开即给推荐 + 支持搜索**：进入「从仓库安装」默认显示「常用推荐」（如实标注「按常见类目拉取，非热度排行」，且注明来源与「无下载量字段」）；搜索框即时按名称/类目/描述过滤，搜索态支持「加载更多」（对应 cursor 翻页）。
- **D10（本轮）冲突走弹窗对比 + 两选**：同名但内容不一致的冲突，卡片上对应客户端标橙「冲突」+ 横幅；点开弹出左右对比（库里定义 vs 客户端现有，仅显示 config + 密钥键名，值永不出后端）。仅两个出口：**「用客户端的（反向入库）」** 与 **「该客户端先跳过」**。**去掉「用我的覆盖」**——它与 AGENTS.md「绝不用库里旧版本覆盖用户改动」红线冲突（用户 2026-09-21 确认去掉）。

## Requirements

- R1：主页面重做为单列表（去三页签），顶栏精简，删客户端筛选条与图例行。
- R2：卡片逐客户端四开关，点击即时应用/撤下（改 `targets` + `save`）；总开关控制启用/停用。
- R3：打开自动扫描，未纳管条目内联展示于主清单下半段，支持就地纳管 + 全部纳管；删独立扫描入口。
- R4：「添加 MCP」弹窗两来源（手动 / 仓库安装）；仓库安装默认推荐列表 + 搜索 + 翻页，来源与「非热度榜/无下载量」如实标注。
- R5：同名冲突弹窗对比 + 两选（反向入库 / 跳过），无「用我的覆盖」；对比只展示 config + 密钥键名。
- R6：数据兼容 —— 存量 `mcp_servers` 与四客户端已应用配置不受损；不改 `xiaobai_` 命名空间、备份/原子/锁、`FINGERPRINT_TABLES`。

## Acceptance Criteria

- 主页面无 tab，单列表呈现；顶栏无客户端筛选条、无图例行。
- 卡片四开关点击后调用 `save_mcp_server`（改 targets），写盘结果回显；总开关停用后从各客户端撤下。
- 页面打开自动扫描；未纳管条目内联显示并可就地纳管/全部纳管；纳管后转为普通卡片。
- 添加弹窗仅「手动/仓库安装」两来源；仓库安装默认给推荐、可搜索、可加载更多，文案标注来源与「非热度榜」。
- 冲突卡片标橙 + 弹窗对比；仅「反向入库/跳过」两选；对比不展示任何密钥值。
- `pnpm typecheck` + `McpPage.test.tsx` 相关用例通过；若动后端则 `cargo test`（src-tauri）通过、`mcp_servers` 仍在 `FINGERPRINT_TABLES`。
- 存量数据与四目标已应用配置抽查无损（上线前真机）。

## Out of Scope

- 新增应用目标客户端（仍为四目标）。
- MCP 协议本身与 server 端运行调试。
- WebDAV 同步/备份协议语义变更（仅要求不受影响）。
- 「用我的覆盖」式的强制覆盖能力（守红线，本轮明确不做；如需另立需求评估）。
- 真实「下载量/热度排行」（注册表无此字段，做不出）。

## Open Questions

- 无阻塞项。实现期一个已知技术项（非产品决策，记 design.md）：冲突弹窗「反向入库」在 `name_conflict`（同名不同身份）场景下会撞 `repo::mcp::save` 重名校验，需在实现时定「改名后入库」或「合并」策略。
