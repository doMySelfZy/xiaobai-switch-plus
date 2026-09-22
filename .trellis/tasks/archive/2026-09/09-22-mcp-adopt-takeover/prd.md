# 纳管即接管

## Goal

把 MCP 的"接管"从**应用时**提前到**纳管时**：纳管一条已存在于多个客户端的 MCP，立即在所有同款客户端里删掉手工条目、写入托管 `xiaobai_<name>` 并置为启用。这样删除该托管条目时能把所有客户端里的 `xiaobai_<name>` 一起清干净，不再回流到"未纳管"列表。

用户价值：修复"纳管后删除，条目仍出现在未纳管列表"的困惑——根因是纳管只写数据库、不碰客户端文件，手工条目一直在，删掉库记录后重扫又被判成未纳管。

## Background / 确认事实（均来自代码勘查）

- **纳管当前只写库、不写盘**：`import_scanned_mcp`（`commands/mcp.rs:634`）把条目存库并按 `scan_target_union` 把 targets 设为所有同款客户端的并集，但注释明确"只设关联、不立即写盘——写盘留给用户之后的应用"（`mcp.rs:646`）。
- **去重与并集已就绪**：`import_entry`（`mcp.rs:603`）粗身份命中已有行则不建重复行（`AlreadyImported`）；`scan_target_union`（`mcp.rs:496`）已把 targets 设为并集；`mark_imported_entries`（`mcp.rs:517`）已按粗身份把同款扫描项标成"已纳管"。**这三项符合目标，保留不动。**
- **接管逻辑已存在于应用路径**：`apply_to_targets`（`mcp.rs:399`）→ `apply_servers_to_targets`（`mcp.rs:318`）→ 各 `mcp_adapters::apply_to_{claude,codex,pi,prime}`。适配器按 AGENTS.md 规则处理"同名未托管条目等价则删除改写 `xiaobai_`，不等价则报错跳过该目标、原样保留文件"，并做备份+原子替换+锁+指纹校验。**纳管即接管 = 纳管后复用这条应用路径。**
- **删除已依赖 applied_targets 清理**：`delete_mcp_server`（`mcp.rs:217`）删库行后 `apply_to_targets(&state, &[])`，靠"上次应用过的目标"（`repo::mcp::applied_targets`）清理 `xiaobai_` 条目。只要纳管时把 targets 记进 applied_targets，删除就能全清。
- **前端纳管后已重扫**：`McpPage.tsx:663` 在 `importScanned` 后 `setScanOutcome(await scanExisting())`；未纳管列表判据是 `!managed && !importedId && !nameConflict`（`McpPage.tsx:612`）。纳管写盘后重扫即会把手工条目变为托管态，从未纳管列表消失。

## Requirements

R1. 纳管后立即接管
- `import_scanned_mcp` 在导入循环结束后，对"本次导入或命中已有行"的记录所涉及的 targets 并集调用 `apply_to_targets(&state, &touched_targets)`，触发对这些客户端的写盘接管。
- 复用现有应用路径，不新写一套接管/备份/锁逻辑。

R2. 接管覆盖所有同款客户端（并集）
- 保留 `scan_target_union` 的并集口径：单条记录的 targets = 所有存在同款粗身份配置的客户端，全部启用。
- 接管在这些 targets 上全部执行：每个客户端里等价的手工条目被删、写入 `xiaobai_<name>`。

R3. 单条托管记录
- 保留现有去重：同款多客户端只建一条记录（`import_entry` 的 `AlreadyImported` 分支不变）。

R4. 删除清干净
- 纳管经 `apply_to_targets` 会把 targets 记入 applied_targets；删除时 `apply_to_targets(&state, &[])` 依赖它把所有关联客户端的 `xiaobai_<name>` 清掉。删除路径本身无需改动，只依赖 R1 让纳管把 applied_targets 记全。

R5. 接管失败的回传与安全
- 某客户端里同名条目已被用户在扫描后改动、与库记录不等价时，适配器**报错并原样保留该客户端文件**（既有行为，不得覆盖用户改动）。
- `import_scanned_mcp` 返回值扩展，携带本次接管的 `McpApplyResult`，前端据此提示哪些客户端接管失败；`imported`/`already_imported`/`failed` 三段语义保持不变（仍只表示入库结果）。
- 安全红线不变：密钥值只由后端按定位符读盘，不经前端；返回值不含 env/headers 明文。

R6. 契约与文档
- 更新 AGENTS.md 中"导入本身不修改来源客户端文件——接管发生在之后的应用时"的表述，改为"纳管即接管：纳管时立即在所有同款客户端接管手工条目"。
- `import_scanned_mcp` 相关注释同步更新。

## Acceptance Criteria

- AC1：同一 MCP 存在于 Claude + Codex 两个客户端，纳管一次后：两个客户端文件里原手工条目消失、出现 `xiaobai_<name>` 且启用；库中只有一条记录，targets = [ClaudeCode, Codex]。
- AC2：接着删除该托管记录：两个客户端里的 `xiaobai_<name>` 均被清除；重新扫描该 MCP 不再出现在任何列表（未纳管、已纳管都没有）。
- AC3：纳管前用户在某客户端把该条目改成与库记录不等价，纳管时该客户端接管失败、文件原样保留，`import_scanned_mcp` 返回的 apply 结果里该目标标记为失败；其余客户端正常接管。
- AC4：`cargo test`（`src-tauri`）通过，含新增/更新的纳管接管端到端用例（用 tempdir 客户端文件断言写盘与清理）。
- AC5：`pnpm typecheck` 与 `pnpm test:run` 通过；前端纳管流程测试覆盖"纳管后未纳管列表清空 + 接管失败提示"。
- AC6：AGENTS.md 契约文字与代码行为一致。

## Out of Scope

- 不改 `scan_target_union` 并集口径、不改 `import_entry` 去重、不改 `mark_imported_entries` 标注（均已符合目标）。
- 不改删除命令本身逻辑（仅依赖纳管把 applied_targets 记全）。
- 不改数据库 schema / 同步指纹表。
- 不新增独立"接管"命令——接管是纳管的一部分，复用应用路径。
