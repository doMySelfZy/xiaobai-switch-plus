# 设计：纳管即接管

## 核心思路

接管逻辑（删等价手工条目、写 `xiaobai_`、备份+原子+锁+指纹校验）已完整存在于应用路径 `apply_to_targets` → 各适配器。本任务不重写这套逻辑，只把它的触发时机从"用户点应用"提前到"纳管完成"。改动集中在 `import_scanned_mcp` 一个命令 + 其返回类型 + 契约文档。

## 数据流

纳管前（现状）：
```
import_scanned_mcp → 存库(targets=并集) → 返回元数据 → [磁盘未动]
用户手动点应用 → apply_to_targets → 适配器接管写盘
```

纳管后（目标）：
```
import_scanned_mcp
  → 逐条 load_entry_for_import + import_entry 存库(targets=并集)   [不变]
  → 收集本次涉及的 targets 并集 touched
  → apply_to_targets(&state, &touched)                          [新增：立即接管写盘]
       → apply_servers_to_targets → 适配器：删等价手工条目、写 xiaobai_、备份/原子/锁/指纹
       → record_applied_targets(记入 applied_targets)            [使删除可清理]
  → 返回 McpImportResult { imported, already_imported, failed, apply }  [扩展 apply 字段]
```

删除（不变，靠 applied_targets 生效）：
```
delete_mcp_server → repo::mcp::delete → apply_to_targets(&state, &[])
  → 并集含 applied_targets → 适配器清掉所有关联客户端的 xiaobai_<name>
```

## touched targets 的计算

`import_scanned_mcp` 循环里对每个 locator：
- `Imported(summary)`：该记录 targets 已是并集，纳入 touched。
- `AlreadyImported`：记录已存在，其 targets 也应纳入 touched（保证重复纳管同款时也会把尚未接管的客户端补接管）。
- `Failed`：不纳入。

实现：循环里累积一个 `HashSet<TargetKind>`。`Imported` 分支直接用 `summary.targets`；`AlreadyImported` 分支用 `existing_id` 回查记录的 targets（或在 import_entry 命中分支里回传 targets）。循环结束后转成 `Vec` 调 `apply_to_targets`。

若 touched 为空（全部 failed）则跳过 apply，直接返回。

## 返回类型扩展

```rust
pub struct McpImportResult {
    pub imported: Vec<McpServerSummary>,
    pub failed: Vec<McpImportFailure>,
    pub already_imported: Vec<AlreadyImportedMcp>,
    pub apply: Option<McpApplyResult>,   // 新增：本次接管写盘结果；无 touched 时为 None
}
```
前端 `McpImportResult` 类型（`src/types/mcp.ts` 附近）同步加可选 `apply`。前端在纳管后读取 `apply.results` 里 `ok == false` 的目标做提示（沿用应用失败的提示口径）。

## 兼容与安全

- **指纹校验兜底不变**：适配器在写前判断同名未托管条目是否与库记录等价（忽略 type/transport 标记）。纳管时配置刚由 `load_entry_for_import` 读盘取得，等价成立 → 干净接管；若用户在扫描后改过导致不等价 → 适配器报错跳过该目标、原样保留文件（AC3）。
- **密钥红线不变**：`import_scanned_mcp` 返回值仍不含 env/headers 明文；`McpApplyResult` 只含 target/ok/backup_paths/message，无密钥。
- **重复加载防护不变**：接管即"删手工 + 写托管"，净结果每个客户端一条，不会两份并存。
- **无 schema / 同步指纹改动**：纳管接管只写客户端文件 + applied_targets（已有表）。

## 失败与回滚

- 部分目标接管失败：记录已入库（targets=并集），失败目标经 `targets_to_record` 仍留在 applied_targets，下次应用/纳管自动重试；成功目标已接管。语义与既有应用失败一致。
- 回滚整个特性：`git revert`；纳管退回"只写库"，手动应用仍可接管。无数据破坏。

## 影响面

- 后端：`commands/mcp.rs`（`import_scanned_mcp` + `McpImportResult`）。
- 前端：`src/types/mcp.ts`（类型）、`McpPage.tsx`（纳管后读 apply 结果做提示，已有重扫保留）。
- 文档：`AGENTS.md`（纳管接管契约）。
- 测试：`commands/mcp.rs` 端到端纳管用例、前端 `McpPage.test.tsx`、`browserMock.ts`（`import_scanned_mcp` mock 需回带 apply 字段）。
