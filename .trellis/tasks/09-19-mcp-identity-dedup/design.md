# 设计：MCP 身份去重

## 不变量（改动前后都必须成立）

1. `mcp_servers` 表结构不变、`FINGERPRINT_TABLES` 不变 → 逻辑指纹算法零变化，不引入跨设备覆盖风险。
   身份**一律读取时计算**，不落库。
2. `repo/*` 目前对 `crate::adapters` **零依赖**（已核实）。这条边界保留：身份逻辑放 adapters，
   去重守卫放 `commands/mcp.rs`（`repo::mcp::save` 的**全部**调用方就在那里的两处，:165 与 :440）。
3. 不改写用户已入库的 `config`：私有字段只在比对时忽略，存储原样。
4. 不删用户客户端文件里的手工条目；写盘仍走「备份 → 原子替换 → `<文件名>.lock`」。

## 新模块：`src-tauri/src/adapters/mcp_identity.rs`

复用 `mcp_scan::canonical`（`pub(crate)`，已做键序归一）与 `mcp_scan::entry_fingerprint` 的既有口径。

```rust
/// 客户端私有字段：比对时剔除，存储不动。
pub const CLIENT_PRIVATE_FIELDS: [&str; 4] = ["type", "transport", "enabled", "timeoutMs"];

/// `npx` / `npx.cmd` / `D:\Program Files\nodejs\npx.cmd` -> `npx`
pub fn normalize_command(command: &str) -> String;

/// 粗身份：同一服务器的不同键名要收敛到同一个值。
pub fn coarse_identity(kind: McpKind, config: &Value) -> String;

/// 等价：粗身份 + env/headers 值。用于「能不能自动合并」。
pub fn equivalence(kind: McpKind, config: &Value, env: &Value, headers: &Value) -> String;
```

`coarse_identity` 的输入 = 剔除私有字段后的 config，其中 `command` 走 `normalize_command`，
其余（`args` 数组、`url` 全文含 query）按 `canonical` 稳定序列化，再带上 `kind`。
`url` 保留 query：`exa` 的 `?tools=...` 是**功能选择**，不是噪声。

`equivalence` = `coarse_identity` + `canonical(env)` + `canonical(headers)`。

### 两套口径必须分开（实现阶段的纠正）

原设想「让 `entry_fingerprint` 成为 `equivalence` 的薄封装，全库一套算法」是**错的**，已纠正：

- `adapters/mcp.rs` 的 `untracked_matches_record` 判断的是「**能不能删掉用户客户端里那条手工条目**」，
  后果是丢数据。AGENTS.md 明确它只忽略 `type`/`transport` 这类显式类型标记。若一并忽略
  `enabled`/`timeoutMs`，用户在 Claude 里主动停用（`enabled: false`）或调过启动超时的条目会被判
  「等价」→ 删除 → 写回库内版本，等于**悄悄把用户禁用的服务重新启用**。
- 本模块的 `coarse_identity` 判断的是「是不是同一个服务器」，后果只是不新建重复行，
  放宽不丢数据，才需要吞掉客户端私有字段。

因此：`entry_fingerprint` 删除（它在 HEAD 上就已无生产调用方，语义被本模块完全覆盖），
接管比对保持原有的窄口径手写比对，`mcp_identity` 是身份口径的唯一实现。
回归钉死：`adapters/mcp.rs` 的 `takeover_refuses_when_user_disabled_or_retuned_entry`。

## 落点

| 位置 | 改动 |
|---|---|
| `commands/mcp.rs:384-391` `scan_existing_mcp` | `imported_id` 先按粗身份匹配，未命中再按名字兜底（保住手工改过名的既有条目） |
| `commands/mcp.rs:425` `import_scanned_mcp` | 命中粗身份 → 不建行，`McpImportResult` 增 `already_imported: Vec<{locator, existing_id, existing_name}>`；`failed` 语义不变 |
| `commands/mcp.rs:159` `save_mcp_server` | 新增行命中他行粗身份 → 返回 `validation_failed`，信息里点名冲突条目；按 id 编辑自身放行 |
| `commands/mcp.rs:263` `apply_to_targets` | 写盘前按粗身份分组，同目标内出现两条启用且同粗身份 → 该目标 `ok=false`，两条都不写（最后防线） |
| `adapters/mcp_version.rs:22` | `command == "npx"` 字面比较改为 `normalize_command(command) == "npx"`，`uvx`/`pnpm` 同理 |
| `McpPage.tsx:892` 前 | 重复项横幅（复用 `ProxyPage.tsx:340-345` 的 `Alert type="warning"` 形状） |
| `McpPage.tsx` 纳管卡片 `:1008-1133` | 「已纳管」Tag 文案带上库内名字；`already_imported` 计入提示 |
| 新命令 `detect_mcp_duplicates` / `merge_mcp_servers` | 见下 |

## 重复检测与合并

`detect_mcp_duplicates() -> Vec<DuplicateGroup>`：
`{ keep_suggestion_id, members: [{id, name, targets, enabled, created_at}], equivalent: bool }`。
按 `coarse_identity` 分组，组内大小 > 1 才算一组；`equivalent` = 组内 `equivalence` 是否全等。
建议保留条：有目标绑定的 → 启用的 → `created_at` 最早的（排序键固定，可单测钉死）。

`merge_mcp_servers { groups: Vec<MergeGroup> }`，`MergeGroup { keep_id, drop_ids }`：

1. 逐组校验：`keep_id` ∈ 该组；若该组 `equivalent == false` 且调用方未显式给出 `keep_id` → 整批拒绝。
   前端对非等价组必须让用户点选后才提交，不预选。
2. `app_backup::create_local_backup(&state.db, "pre_mcp_merge", max_copies)` —— 先落整库快照再动数据。
3. 组内重读 `list_full` 复核粗身份仍然相同（防检测与合并之间被同步换库插队）；变了就跳过该组并报错。
4. `keep` 行的 `targets` 取并集、`enabled` 取「任一启用」，其余字段不动；`drop` 行删除。
5. 全部组处理完再统一调 `apply_to_targets(&state, &[]?)`：靠 `mcp_applied_targets` 的并集语义把
   被丢弃条目的 `xiaobai_<name>` 从各客户端扫掉。
6. 返回逐组结果（合并了几组、清理了哪些目标、哪些失败），失败不回滚已成功组（快照已兜底），
   但必须逐条报给界面。

## 同步侧（C）

- 身份是读取时算的，所以换库后检测天然生效，无需迁移钩子。
- 启动路径（`state.rs:22` `apply_pending_restore`）**不做任何自动合并**：整库替换可能把对端机器
  新建的重复行带进来，静默删行等于删别人的数据。只在面板打开时检测并提示。
- `mcp_applied_targets` 存在 `sync_meta`（不参与指纹、不跨机同步）是**正确的**：它描述的是本机文件里
  写过什么。但要知道其后果——换库后本机陈旧的 `xiaobai_*` 要等下一次 apply 才被扫掉；
  合并动作显式触发 apply，正好覆盖这个窗口。

## 取舍

- **为什么不落库存身份列**：加列会改 `SELECT *` 的指纹输入，等于改指纹算法，需要递增
  `FINGERPRINT_ALGORITHM_VERSION` 并让两端同时升级 —— 收益（省一次哈希）远小于风险。
- **为什么纳管命中不自动并入 targets**：会在用户没要求的情况下改已有行；用户要的是别建重复行，
  不是替他做决定。
- **为什么不自动合并非等价组**：本机 `ssh` 那对 args 真不同（一条带 `--whitelist`），
  自动合并必丢配置。
- **为什么保留名字匹配作兜底**：既有库里用户手工改过名的行，粗身份可能因 args 微调而不等，
  纯身份匹配会把「已纳管」显示成未纳管，反而诱导重复纳管。

## 回滚

纯新增命令 + 判定收紧，无 schema 变更、无协议值变更。回滚 = revert 提交。
唯一不可自动逆的是合并动作本身，靠 `pre_mcp_merge` 快照 + 既有一键恢复回退。
