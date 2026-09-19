# 退役一个已落库的枚举值 / 应用目标

> 场景：从 `TargetKind` 这类**已经写进用户数据库**的 serde 枚举里删掉一个取值。
> worked example：ZCode（任务 `.trellis/tasks/09-19-remove-zcode-target`，模块 `src-tauri/src/zcode_retirement.rs`）。
>
> 「退役完成」的判据**不是** grep 不到那个词，而是：老版本用户的库升级后还能读、还能存，
> 且本应用当年写进他们机器的痕迹被收口。按这个判据，删代码只是最后一步。

---

## 1. Scope / Trigger

触发条件（满足任一即按本 spec 走）：

- 删/改 `src-tauri/src/domain/mod.rs` 的 `TargetKind` 变体，或任何 `Vec<TargetKind>` 持久化字段；
- 删 `sites` 等有真实存量的表的某一列的**读写代码**；
- 删某个适配器（`adapters/*.rs`）而它曾在用户机器上写过文件。

**硬顺序**：数据清洗层必须先于枚举删除并发布在一起。反序的后果不是报错，是
**打不开老用户的库**（见 §4 的失败模式）——而中间态版本可能已经被别人构建出来。

## 2. Signatures

```rust
// 数据库侧：一次性、幂等，挂在既有增量补齐入口，**不**新增 SCHEMA_VERSION
crate::zcode_retirement::ensure_in_db(&Connection) -> AppResult<DbChanges>
// 挂载点：db/migrate.rs::ensure_incremental_schema 的末尾（依赖前面的增量补齐先把表/列备好）

// 文件侧：启动时跑一次
crate::zcode_retirement::leftover_home_override(&Connection) -> AppResult<Option<String>>
crate::zcode_retirement::clean_external_once(Option<&str>) -> FileReport
// 挂载点：lib.rs 的 setup，AppState::init() 之后、托盘与本地代理之前（不要放进 apply_schema）
```

`DbChanges::is_empty()` 用于区分「什么都没动」，`FileReport { written, skipped }` 同理。

## 3. Contracts

**两种拼写都要认。** `#[serde(rename_all = "snake_case")]` 在每个大写前插 `_`，
所以 `ZCode` 落库是 `"z_code"`；而手写的 `TargetKind::as_str()` 产出 `"zcode"`。
真实存量里**两种都有**（数组元素走 serde 通道，行表列值走 `as_str()` 通道）：

```rust
const RETIRED_JSON_NAMES: [&str; 2] = ["z_code", "zcode"]; // settings/localProxyTargets、targets_json、sync_meta 键
const RETIRED_COLUMN_VALUES: [&str; 2] = ["zcode", "z_code"]; // target_bindings.target、apply_records.target
```

**清洗器自包含。** 它只认识字符串：退役目标的字面量、`xiaobai_` 前缀、托管块标记、
落点相对路径。不 import 被删的适配器，不引用被删的枚举变体，路径常量也不从 `paths.rs` 暴露。
否则 U2/U3 删完它自己就编译不过了。

**列留值空。** `sites.zcode_api_type` 的值置 `NULL`，物理列与 DDL 一律不动：
`sync.rs` 的逻辑指纹走 `SELECT * FROM {table} ORDER BY rowid` 逐列哈希，
**列集合是算法的输入**，改列集等于改算法（见 [webdav-sync](./webdav-sync.md) 不变量 3）。

**已删字段的历史键还能捞回来。** `AppSettings` 是宽松反序列化（无 `deny_unknown_fields`），
所以删掉 `zcodeHomeOverride` 结构体字段后，该键仍留在 `settings.json` blob 文本里，
直到用户第一次 `save_settings`。要读它就直接读原始 JSON（`leftover_home_override` 即为此存在），
**不要**为此把字段加回生产结构体——那等于把退役目标又请回来一遍。

## 4. Validation & Error Matrix

| 条件 | 行为 | 为什么 |
|------|------|--------|
| 清洗计划为空 | 一个写都不发、不备份 | 干净库不该凭空多备份 |
| 清洗过程出错 | 挂载点降级为 `warn`，下次启动重试 | 冒泡出去会变成「数据库打不开」，用户连应用都用不了 |
| 托管块只有 BEGIN 没有 END / 非 UTF-8 / 路径是目录 | 报错并**原样保留文件** | 与 `agents.md` 既有约定一致：形状不合法就不覆盖 |
| 历史 override 键缺失或为空白 | 视为 `None` | **绝不**静默回退到真实 `~/.zcode`，那是用户的目录不是我们的 |
| 默认根与 override 根是同一目录 | 只清一次（`canonicalize` 去重） | 否则同一文件被写两遍、备份两遍 |
| 落点目录不存在 | 全程静默，不创建任何文件 | 没用过这个目标的人不该感知本次升级 |
| 未知/退役目标的绑定行 | 跳过并 `warn`，**不重映射** | 见 §7 |

## 5. Good / Base / Bad Cases

- **Good**：v0.1.5 形状的库升级 → 设置页可读可存、MCP 列表读得出、四个目标的全局约束托管块与
  MCP 托管条目仍能正常清理；用户机器上假 `~/.zcode` 里我们的条目被摘掉、块外内容逐字节保留（含 BOM）。
- **Base**：全新库 / 从未勾选过该目标 → 全程 no-op，不写盘不备份。
- **Bad（都是真实踩过的）**：
  1. 先删枚举再考虑数据 → `TargetKind` 无 `#[serde(other)]`，未知变体让**整份**文档反序列化失败；
  2. `TargetKind::parse(&s).unwrap_or(TargetKind::ClaudeCode)` → 脏行伪装成 Claude Code，
     删站点时走进 `claude_code::surgical_revert`，按 `key_fingerprint` 匹配可能**误删用户
     `~/.claude/settings.json` 里的真实鉴权键**；
  3. `serde_json::from_str::<Vec<TargetKind>>` 一把梭 → 一条脏元素让整份列表不可读；
  4. `unwrap_or_default()` 解析目标数组 → 脏数据把**全部**目标的清理集合静默清空。

## 6. Tests Required

守门测试要**先红后绿**：先造夹具写断言（此时实现还不存在，必须看到失败），再实现。

- 夹具两种拼写都要有；行表列值与 JSON 数组元素分别造。
- 必覆盖的断言点（ZCode 实际 27 例）：幂等 + 只备份一次、未知目标被跳过而非冒充、
  托管条目被摘而用户条目存活、畸形文件原样保留、无我们的东西就不写盘、
  空白 override 不回退真实目录、历史 override 键在清洗后仍可读回、同一根只清一次。
- **变异验证**（逐条把实现改坏，确认对应用例变红）至少做：`map_binding` 退回 `unwrap_or(默认)`、
  托管前缀判断去掉、清理改成 no-op、备份挪到写盘之后、缺 END 时从 `Err` 改成截断。
- 断言只能被**加强**不能被放宽。确因架构变化而失效的前置条件，改写成新不变式
  （例：ZCode 的 MCP 用例把「库里还留着 zcode」这一前提反转成「清洗一定先于写盘」的顺序断言），
  并在提交说明里写明理由。

## 7. Wrong vs Correct

```rust
// Wrong —— 编译通过，静默改变语义
let kind = TargetKind::parse(&row.target).unwrap_or(TargetKind::ClaudeCode);
let targets: Vec<TargetKind> = serde_json::from_str(&targets_json)?;   // 一条脏行 = 整表不可读
conn.execute_batch("ALTER TABLE sites DROP COLUMN zcode_api_type")?;    // 改了指纹算法输入

// Correct
let kind = TargetKind::parse(&row.target);            // Option：None 就跳过并 warn
let targets = domain::parse_persisted_targets(&targets_json, "mcp"); // 逐元素 + warn + skip
// 列保留，只把值置 NULL；读写侧都不再引用它
```

前端同理：目标 → i18n key 的映射函数**不要**给默认臂指向某个存活目标，
用 `const unreachable: never = kind; return unreachable;`（见
[frontend/type-safety](../frontend/type-safety.md)）。

## 何时可以整删这个模块

三条同时成立才删（原文在 `zcode_retirement.rs` 文件头）：升级窗口结束、
跨机混跑窗口结束（对端不会再推旧数据）、产品接受用户机器上可能的残留。
