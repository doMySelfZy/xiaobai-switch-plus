# 设计：移除 Prime 应用目标

## 硬顺序（不可颠倒）

按 `target-retirement.md` §1：**数据清洗层必须先于枚举删除，并在同一批发布**。反序会打不开老用户的库。本任务一个提交批次内完成，但代码落地顺序：

1. 写 `prime_retirement.rs` + 守门测试（此时 `TargetKind::Prime` 仍在，测试先红后绿）。
2. 挂载清洗器到 `db/migrate.rs` 与 `lib.rs`。
3. 最后删 `TargetKind::Prime` 变体、`adapters/prime.rs`、UI、i18n、文档——编译器用穷尽 match 逼出所有残留 arm。

## `prime_retirement.rs` 模块设计（镜像 `zcode_retirement.rs`）

自包含，只认字符串字面量，不 import 被删的 `prime` 适配器、不引用 `TargetKind::Prime`。

```rust
const RETIRED_JSON_NAMES: [&str; 3] = ["prime", "prime_agent", "prime-agent"]; // targets_json / sync_meta / localProxyTargets 数组元素
const RETIRED_COLUMN_VALUES: [&str; 3] = ["prime", "prime_agent", "prime-agent"]; // 若有行表列值（apply_records.target / target_bindings.target）
const RETIRED_HOME_OVERRIDE_KEY: &str = "primeAgentDirOverride"; // settings blob 键
// 用户机器痕迹：~/.prime/agent 下 mcp settings.json 的 xiaobai_ 条目、AGENTS.md 托管块、auth/models 里的 xiaobai_ provider
```

- `ensure_in_db(conn) -> AppResult<DbChanges>`：
  - settings blob：解析原始 JSON，删 `primeAgentDirOverride` 键、`localProxyTargets` 数组里的 prime 元素，写回（仅当有变化）。
  - `mcp_servers.targets_json` / `agent_rules.targets_json`：逐行解析数组，剔除 prime 元素，写回（保留其它目标；空数组保留为空）。
  - `sync_meta` 的 `mcp_applied_targets` / `agent_rules_applied_targets`（或等价键）：剔除 prime。
  - 幂等：再跑一次 `DbChanges::is_empty()`。挂 `ensure_incremental_schema` 末尾，不升 `SCHEMA_VERSION`。
- `leftover_home_override(conn) -> AppResult<Option<String>>`：读原始 settings JSON 的 `primeAgentDirOverride`（结构体字段删除后仍在 blob）。空白视为 `None`，**不回退真实 `~/.prime`**。
- `clean_external_once(override) -> FileReport`：解析目标目录 = override（经 `leftover_home_override`）→ 否则 `~/.prime/agent`（历史默认，用字面量拼，不从 paths.rs 拿）。对该目录下：
  - `settings.json`（Prime 的 MCP 落点，`mcpServers` 键）：摘 `xiaobai_` 前缀条目，用户条目保留；
  - `AGENTS.md` / `CLAUDE.md`（全局约束落点）：摘 `<!-- xiaobai-switch:begin global-rules -->…end -->` 托管块，块外逐字节保留（含 BOM）；文件只剩空白则删（避免空 AGENTS.md 遮蔽 CLAUDE.md）；
  - `auth.json` / `models.json`：摘 `xiaobai_` provider。
  - 备份到 `~/.xiaobai-switch/backups/prime-retirement/` 后原子替换 + 锁；无变化不备份不写盘；畸形（只有 BEGIN 无 END / 非 UTF-8 / 路径是目录）报错原样保留；同一根 canonicalize 去重。
  - 挂 `lib.rs` setup，`AppState::init()` 后、托盘/本地代理前；出错降级 `warn` 下次重试。

## 删除策略（R3）

删除靠 Rust 编译器 + tsc 的穷尽性兜底：先删 `TargetKind::Prime` 变体和 `adapters/prime.rs`，然后逐个编译错误消掉 arm/import/字段。前端删 TS 联合类型的 `"prime"` 后，tsc 会在所有 `switch`/映射处报错，逐一处理；目标→i18n key 映射保持 `const unreachable: never = kind` 兜底（`frontend/type-safety.md`），不给默认臂指向存活目标。

按子代理勘查的清单（见 prd R3）逐文件处理。要点：
- `commands/skills.rs`：`SkillTarget::ALL` 数组长度 `[SkillTarget; 5] → 4`。
- `domain/thinking.rs`：`is_thinking_target` 去掉 Prime；能力矩阵与 `prime_anthropic_rejects_extended` 业务规则一并删。
- `local_proxy/routing.rs`：`TargetKind::Pi | TargetKind::Prime => Anthropic` 改为只 `Pi`。
- `tray.rs` / `tray_apply.rs`：删 `TrayLabels.prime`、`TraySnapshot.prime_*`、`hydrate_prime`、菜单项、tooltip 分支。
- `backup.rs`：删 `"prime-"` id 前缀、文件名映射、auth 权限特判里的 Prime。

## 兼容红线

- 不动 DDL、不动 `FINGERPRINT_TABLES` 列集合（列即指纹算法输入）。
- 不删用户机器 `~/.prime` 目录本身，只摘我们写的托管块/条目。
- WebDAV/备份协议值不变。

## 验证策略

- 后端：`cargo test`（退役守门测试先红后绿 + 变异验证；其余全绿、match 穷尽）。
- 前端：`pnpm typecheck`（tsc 逼出所有 `"prime"` 残留）+ `pnpm test:run`。
- 残留扫描：AC3 grep（排除 `.trellis/`、`prime_retirement.rs`、`.mimosa/`）。

## 回滚

纯移除 + 一次性清洗（幂等、只摘我们写的东西）。回滚 = `git revert` 整批。清洗已发生的数据无法"退役回 prime"，但那正是目标；用户机器文件有备份。
