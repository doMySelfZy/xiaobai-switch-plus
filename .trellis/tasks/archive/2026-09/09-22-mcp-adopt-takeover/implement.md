# 执行计划：纳管即接管

工作流：inline（改动集中在 import_scanned_mcp + 返回类型 + 前端提示 + 文档 + 测试，已完全定位）。

## 有序清单

### 阶段 A — 后端接管触发
1. `commands/mcp.rs`：`McpImportResult` 增加 `pub apply: Option<McpApplyResult>` 字段。
2. `import_entry` 的 `AlreadyImported` 分支回带命中记录的 `targets`（用于 touched 计算），或在 `import_scanned_mcp` 里按 `existing_id` 回查 targets——择简。
3. `import_scanned_mcp`：循环中累积 `HashSet<TargetKind>` touched（`Imported` 用 summary.targets，`AlreadyImported` 用命中记录 targets，`Failed` 跳过）。
4. 循环后：touched 非空则 `let apply = Some(apply_to_targets(&state, &touched_vec)?)`，否则 `None`；组装进返回值。
5. 更新 `import_scanned_mcp` 顶部注释（去掉"导入不写盘"，写明"纳管即接管所有同款客户端"）。
6. `cargo test`（`src-tauri`）。

### 阶段 B — 前端类型与提示
7. `src/types/mcp.ts`：`McpImportResult` 加可选 `apply?: McpApplyResult`（确认 `McpApplyResult` 类型已存在，复用）。
8. `McpPage.tsx`：纳管处理里读取 `result.apply?.results`，对 `ok === false` 的目标用现有应用失败提示口径 message.warning/error；保留纳管后 `scanExisting()` 重扫。
9. `src/lib/browserMock.ts`：`import_scanned_mcp` mock 分支回带 `apply`（模拟接管成功的 McpApplyResult），使前端测试可断言提示分支。

### 阶段 C — 测试
10. `commands/mcp.rs` 测试模块：新增端到端用例（tempdir 造 Claude+Codex 客户端文件各含同款手工条目 → 纳管一次 → 断言两文件手工条目消失、出现 xiaobai_ 且启用、库一条记录 targets 为并集、applied_targets 记全）。
11. 新增删除清理用例：接上一步 → 删除 → 断言两客户端 xiaobai_ 均被清、重扫无残留（AC2）。
12. 新增不等价跳过用例：某客户端条目改成不等价 → 纳管返回的 apply 里该目标 ok=false、文件原样保留（AC3）。
13. 前端 `McpPage.test.tsx`：纳管后未纳管列表清空；接管失败时出现提示。
14. 复核既有 import 相关用例（`import_hitting_coarse_identity...` 等）是否因返回类型扩展需要调整断言。

### 阶段 D — 文档与验证
15. `AGENTS.md`：改写"已有 MCP 的扫描与纳管"一节里"应用时先接管"的表述为"纳管即接管"，保留不等价报错跳过、指纹兜底、密钥不经前端等红线。
16. `cargo test` + `pnpm typecheck` + `pnpm test:run` 全绿。
17. 手动核对（可选）：真机纳管一条多客户端 MCP → 删除 → 未纳管列表不回流。

## 验证命令

```bash
cd src-tauri && cargo test
pnpm typecheck && pnpm test:run
```

## 风险文件 / 回滚点

- `import_scanned_mcp`：touched 计算漏掉 `AlreadyImported` 会导致重复纳管同款时未接管的客户端补不上 —— 用例 11/12 覆盖。
- 返回类型扩展：`apply` 设为 `Option` 且可选，避免破坏既有 `imported/failed/already_imported` 的消费方。
- 密钥红线：确认扩展后的返回值链路（含 apply）不含 env/headers 明文。
- 回滚：纯增量，`git revert` 即退回"纳管只写库"。
