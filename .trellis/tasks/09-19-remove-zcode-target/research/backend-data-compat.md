# Research: 移除 ZCode 目标 —— 后端存量数据与兼容性影响

- **Query**: ZCode 目标移除对 Rust 后端的 schema / 反序列化 / 清理能力 / 硬编码扩散 / 预置模板 / 测试 / 兼容红线的影响
- **Scope**: internal（代码 + git 历史），未跑 `cargo`（按任务要求）
- **Date**: 2026-09-19
- **前提**（主 agent 已确认，未重复调查）：`21782f0` 引入 ZCode，已在 `main`，已随 `v0.1.5` 发布给真实用户。全仓 61 个源文件命中。

---

## 结论摘要

1. **最危险的不是编译错误，是「字符串读不回枚举」。** `TargetKind` 用 serde 派生（`domain/mod.rs:73-82`，`rename_all = "snake_case"`），删掉 `ZCode` 变体后，任何存量 `"zcode"` 字符串都会让反序列化失败。失败在四个不同位置有 **四种不同形态**，其中三种是静默的：

   | 存量位置 | 读路径 | 删变体后的行为 | 危害 |
   |---|---|---|---|
   | `settings.json` blob 里的 `localProxyTargets: [..., "zcode"]` | `repo/settings.rs:32-40` 整体 `from_str(...)?` | **硬 Err，整个 AppSettings 读不出来** | 全局瘫痪（见下） |
   | `mcp_servers.targets_json` 含 `"zcode"` | `repo/mcp.rs:55` `from_str(...)?` | **硬 Err**（`read_row` 用 `?`） | MCP 列表/应用/扫描全死，不止 zcode 那条 |
   | `agent_rules.targets_json` 含 `"zcode"` | `repo/rules.rs:44` `unwrap_or_default()` | **静默变 `[]`** | 全局约束看起来「没配」，且**五个目标都清不掉** |
   | `sync_meta.agent_rules_applied_targets` / `mcp_applied_targets` | `repo/rules.rs:72`、`repo/mcp.rs:220` `unwrap_or_default()` | **静默变 `[]`** | 清理目标集 = 本次勾选 ∪ `[]`，托管块/托管条目永久残留 |
   | `target_bindings.target = 'zcode'` | `repo/binding.rs:12` `unwrap_or(TargetKind::ClaudeCode)` | **静默伪装成 Claude Code** | 交叉污染 / 误删真 claude_code 行 |
   | `apply_records.target = 'zcode'` | `repo/apply.rs:88` → `ApplyRecordDto.target: String`（`domain/mod.rs:974`） | 原样透传 | 无害（前端显示未知标签） |
   | `site_thinking_presets.target = 'zcode'` | `repo/thinking.rs:13-14` 用 `target.as_str()` 进 WHERE | 永不命中 | 无害死行，但仍进逻辑指纹 |

   **确证**：以上均为直接读码所得；serde 单元变体数组的失败语义（一个元素未知 → 整个 `Vec` 反序列化失败）是 serde 标准行为。验证方法：在临时分支删掉 `domain/mod.rs:81`，写一个 `serde_json::from_str::<Vec<TargetKind>>(r#"["claude_code","zcode"]"#)` 的单测 —— 预期 5 个失败点里有 3 个走 `unwrap_or_default`/`unwrap_or`，2 个走 `?`。

2. **`settings` blob 是全局单点故障，且用户无法自救。** `AppSettings` 整份存成一个 JSON（`migrate.rs:41-44` 的 `settings` 表 + `repo/settings.rs:33`），`local_proxy_targets: Vec<TargetKind>`（`domain/mod.rs:612-614`）带 `#[serde(default)]` —— 但 `default` 只在**字段缺失**时生效，字段在但值非法会让整份 `AppSettings` 解析失败（`repo/settings.rs:37-38` 直接 `?`）。而 `src/pages/ProxyPage.tsx:10` 的 `const TARGETS: TargetKind[] = ["claude_code","codex","pi","prime","zcode"]` 证明确实**能在接管列表里勾上 ZCode**，即 v0.1.5 真实用户机器上存在这个状态。**确证：可达。**
   后果链（`get_settings` 共 64 处调用）：
   - `lib.rs:77-80` `.unwrap_or_else(|_| "zh-CN")` → 语言回退（可容忍）
   - `lib.rs:99-107` `let Ok(settings) = settings else { return; }` → **本地代理不再自启**，而客户端配置里的 Base URL 仍指向本地代理 → 用户的 CLI 连不上
   - `autostart.rs:80-85` `.unwrap_or(false)` → **开机自启静默关闭**
   - `commands::get_settings`（`commands/settings.rs:9-12`）→ 前端设置页打不开
   - `commands::save_settings`（`commands/settings.rs:39`）第一步就 `get_settings` → **连保存都失败，UI 里无法自我修复**
   - `commands/targets.rs:38` 第一步读 settings → 主界面目标状态全部报错
   → 结论：**这是"升级后应用变砖、且无 UI 逃生口"级别的问题，必须先做数据清洗再删枚举。**

3. **直接删 `zcode.rs` 会让 `~/.zcode` 下的存量条目变成永久孤儿。** 详见 §3。

4. **与当前分支 `feat/site-add-presets` 不冲突。** 站点预置模板是纯前端，零 zcode 键；`SiteCapabilities` 是 `HashMap<String, bool>`（`src-tauri/src/capabilities.rs:12`）自由键，没有枚举需要改；`zcode_api_type` 是 `sites` 的独立列而不是能力键。详见 §5。

5. **推荐路线（两阶段，成本远低于风险）**：先发布一个「只读兼容 + 数据清洗」版本（保留 `TargetKind::ZCode` 变体与三个 `paths::zcode_*` 路径函数，撤掉所有写入入口），等存量收敛后再删枚举。详见 §3 末。

---

## 1. Target 枚举与序列化

### 1.1 定义（确证）

`src-tauri/src/domain/mod.rs:73-104`：

```rust
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum TargetKind {
    ClaudeCode, Codex, Pi,
    #[serde(alias = "prime_agent", alias = "prime-agent")]
    Prime,
    ZCode,                 // :81
}
```

- 落库字符串 = `as_str()`（`:85-93`）→ `ZCode => "zcode"`（`:91`）；serde 序列化同为 `"zcode"`（snake_case）。
- `parse()`（`:94-103`）：`"zcode" => Some(Self::ZCode)`（`:100`），未知返回 `None`。
- **对比 Prime 的先例**：`Prime` 有 `#[serde(alias = ...)]`（`:79`）+ `parse` 接受三个字符串（`:99`），说明本仓库对「同一目标多种历史写法」已有双识别机制可复用。ZCode 移除后若走「保留解析、删除写入」路线，可以照抄这个形状反向用（保留 `parse("zcode")` 但不再进目标清单）。

### 1.2 存在三个互相独立的目标枚举（确证，容易漏）

| 枚举 | 位置 | ZCode | 说明 |
|---|---|---|---|
| `TargetKind` | `domain/mod.rs:75-82` | 有 | 主枚举，落库 + 前端契约 |
| `ScanTarget` | `adapters/mcp_scan.rs:33-39` | 有（`:38`） | **独立**的 MCP 扫描枚举，`json_servers_key` 只为它特判 `"mcp.servers"`（`:176-179`） |
| `SkillTarget` | `commands/skills.rs:42-48` | **无** | 技能安装本来就不含 ZCode，无需改动 |

### 1.3 必须改动才能编译的 exhaustive match（无 `_` 兜底）

**确证**，逐个核对过 arm 覆盖：

| # | 位置 | 用途 |
|---|---|---|
| 1 | `adapters/agent_rules.rs:219-231` | `target_path`：ZCode → `agents_or_claude(resolve_zcode_home(...))`（`:227-230`） |
| 2 | `commands/targets.rs:67-116` | `detect_status`，ZCode 分支含**孤儿启发式**（`:100-115`：无绑定时扫 `xiaobai_` 前缀 provider） |
| 3 | `commands/targets.rs:119-135` | `live_summary` |
| 4 | `commands/targets.rs:145-170` | `config_path` |
| 5 | `commands/targets.rs:232-266` | 撤销/解绑（`surgical_revert` 派发） |
| 6 | `commands/apply.rs:764-800` | `revert_target` |
| 7 | `commands/apply.rs:832-866` | **`restore_official_target`** —— 兼容红线相关 |
| 8 | `commands/mcp.rs:280-306` | MCP `apply_to_targets` |
| 9 | `commands/sites.rs:199-233` | 删站点带清理 |
| 10 | `key_switch.rs:290-336` | 换 key 时逐目标重写 |
| 11 | `route_switch.rs:104-146` | 换 base_url 时逐目标重写（ZCode 分支最特殊：手工造 `RewriteOutcome`，`backup_paths` 靠 `payload_files` 反推，`:136-140`） |
| 12 | `tray.rs:97-103` | `target_label`，ZCode → `labels.zcode` |
| 13 | `local_proxy/routing.rs:65-71` | `use_claude`：ZCode 按 `zcode_api_type`/站点协议判定 |
| 14 | `adapters/mcp_scan.rs` 内 3 处 ScanTarget match | `:168-172`、`:176-179` 有 `_` 兜底（删变体后**兜底臂接管，编译通过但语义反转** —— 见 §3.2），`:325-332`、`:357-396` 无兜底 |

**不需要改 match 但改了会静默变行为**（if-else 链 / 数组，编译期无保护）：

- `backup.rs:186-199` `parse_backup_id` 的 `else if id.strip_prefix("zcode-")`（`:192-193`）
- `backup.rs:331-337` `restore_backup_in` 的权限位判断（ZCode 两份配置含明文 apiKey）
- `backup.rs:371-376` `summary_from_backup_dir` → `zcode::backup_summary`
- `backup.rs:240-266` `mapped_dest` 的 `(TargetKind::ZCode, "config.json" | "provider_config.json")`（此 match **有 `_ => None`**，删臂后编译通过 → 备份还原静默变成「no restorable files」错误）
- `commands/apply.rs:396-432` `if target == TargetKind::ZCode { ... }`
- `tray_apply.rs:335-365` `pick_tray_targets`（5 个 bool 参数，顺序即目标顺序）
- `tray.rs:648-649` `"status_zcode" | "status_zcode_model" => {}` 空臂

### 1.4 非 TargetKind 的连带结构（确证）

- `TrayLabels.zcode`（`tray.rs:30`，两份语言各一份 `:53`、`:73`）
- `TraySnapshot.{zcode_line, zcode_model}`（`tray.rs:199-200`，`placeholder()` 里 `:231-233`、`:248-250`）
- `format_tooltip` **6 参数**（`tray.rs:149-153`，`labels.header, claude, codex, pi, prime, zcode`）
- `TargetOverrides.zcode_home`（`agent_rules.rs:236-242`）、`overrides_from` 里 `zcode_home: settings.zcode_home_override.clone()`（`commands/rules.rs:19`）
- `SiteRow.zcode_api_type`（`domain/mod.rs:1093-1094`，`SiteDto:165-167`、`CreateSiteInput:216-218`、`UpdateSiteInput:286-288`）
- `AppSettings.zcode_home_override`（`domain/mod.rs:585-587`，默认值 `:901`）
- `SiteRow::to_dto` 的 `zcode_api_type: self.zcode_api_type.clone()`（`domain/mod.rs:1163`）

---

## 2. 数据库 schema 与迁移

### 2.1 存目标值的表/列（确证，逐列核对 `migrate.rs`）

| 表 | 列 | 定义位置 | 目标值形态 | CHECK / FK / 索引 |
|---|---|---|---|---|
| `target_bindings` | `target` | `migrate.rs:114` | 裸字符串 `'claude_code'`/`'zcode'` | **TEXT PRIMARY KEY**；`idx_target_bindings_orphan`（`:182`）；FK `site_id → sites ON DELETE SET NULL`（`:129`） |
| `apply_records` | `target` | `migrate.rs:137` | 同上 | `idx_apply_records_target_time ON (target, applied_at DESC)`（`:181`）；FK `site_id`（`:149`） |
| `site_thinking_presets` | `target` | `migrate.rs:107` | 同上 | 复合 PK `(site_id, target)`（`:110`）、FK `site_id ON DELETE CASCADE`（`:106`） |
| `mcp_servers` | `targets_json` | `migrate.rs:25` | **JSON 数组** `["claude_code","zcode"]` | 无 |
| `agent_rules` | `targets_json` | `migrate.rs:37` | JSON 数组，默认 `'[]'` | `id INTEGER PRIMARY KEY CHECK (id = 1)`（`:35`） |
| `sync_meta` | `agent_rules_applied_targets` / `mcp_applied_targets` | `repo/rules.rs:8`、`repo/mcp.rs:11` | JSON 数组（KV 表） | key PRIMARY KEY |
| `settings` | `json` | `migrate.rs:41-44` | **整份 AppSettings JSON**，内含 `localProxyTargets: ["...","zcode"]` 与 `zcodeHomeOverride` | `CHECK (id = 1)` |
| `sites` | `zcode_api_type TEXT` | `migrate.rs:61`（新库）、`:204-211`（存量库 ALTER） | **不是目标值**，是协议字符串（`anthropic-messages` 等） | 无 |

- **没有任何一个 target 列带 CHECK 约束**（`migrate.rs` 里所有 `CHECK` 都是 `id = 1` 单行表守卫，见 `:35/:42/:154/:168/:766`）。→ 好消息：不需要为约束重写表；坏消息：**数据库不会拒绝任何未知目标字符串**，脏值只能靠代码容忍。
- `apply_schema` 的三个「库已存在」分支（`migrate.rs:295-299` 版本达标早退、`:301-313` legacy、`:315-319` 全新）都跑了 `ensure_incremental_schema`（`:280-287`），符合 agents.md 约定。若删除 `zcode_api_type` 列，必须同时处理 `ensure_sites_newapi_columns`（`:204-211`）与 `V1_SCHEMA`（`:61`），否则新老库列集不一致。

### 2.2 `zcode_api_type` 的两难（确证）

`repo/site.rs:89` 的 `SITE_SELECT` **显式列出了 `s.zcode_api_type`（末列）**，`repo/site.rs:52` 用 `row.get(23).ok().flatten()` 读它。

- 只删 DTO 字段不删列：可行，`.ok()` 本来就容错。
- 同时删 `SITE_SELECT` 里的这一项：**必须把 index 23 及其后所有位置重编号**（当前 23 是最后一列，所以只需去掉 SELECT 尾项 + `:52` 一行，风险低）。
- **但删列本身（`ALTER TABLE ... DROP COLUMN`）会改逻辑指纹**，见 §2.3 —— 强烈建议**列保留不动**。
- 写入路径三处：`repo/site.rs:187`（INSERT 列名表）、`:209`、`:327-335`（update 里空串转 None）、`:343`（UPDATE `zcode_api_type=?16`）、`:360`。

### 2.3 FINGERPRINT_TABLES 与跨机同步（确证，重要）

`sync.rs:41-51`：9 张表，含 `settings`、`sites`、`site_thinking_presets`、`target_bindings`、`apply_records`、`mcp_servers`、`agent_rules`。`FINGERPRINT_ALGORITHM_VERSION: u32 = 1`（`:56`）。

`compute_logical_fingerprint`（`sync.rs:73-115`）对每张表跑 **`SELECT * FROM {table} ORDER BY rowid`**，逐列逐值哈希。→ **列集合和行集合都是指纹的一部分**。

推论：
- `FINGERPRINT_TABLES` 本身**不需要动**（ZCode 不是表）。测试 `sync.rs:733-734` 断言 `FINGERPRINT_TABLES.len() == 9`，只要不增删表就不会红。
- **删 `sites.zcode_api_type` 列 = 改变列集合 = 改变指纹**。而 `sync.rs:53-61` 的注释明确记录了历史事故：0.1.3 的 8 表与 0.1.4 的 9 表对同一数据算出不同指纹，因 `algorithm_matches`（`:65-67`）判定「都是 v1」而**恒定互相覆盖**。删列会 100% 复现同一类缺陷（一台升了、一台没升）。
- 同理，一次性 `DELETE FROM target_bindings WHERE target='zcode'` 也会改指纹 —— 但那是**行数据变更**，语义上等价于「用户删了一条绑定」，两端最终会收敛（一次 push）。可接受。
- 同步是**整库文件替换**：`sync.rs:385-424` 下载 bundle → `validate_and_extract_bundle` → 指纹比对 → `queue_pending_restore`，下次启动换库。**确证**：只要对端还跑 v0.1.5 且勾了 ZCode 接管，本机升到「已删 ZCode 变体」的版本后换库就必然读不出 settings。→ **版本偏斜窗口内不能删枚举。**

### 2.4 读取路径失败模式（确证，含定位）

| 失败模式 | 位置 | 说明 |
|---|---|---|
| 硬 Err | `repo/settings.rs:37-38` | 见摘要 §2 |
| 硬 Err | `repo/mcp.rs:53-55`（`targets: serde_json::from_str(&targets_json)?`） | `read_row` 被 `list()`（`:70-77`）与 `get()` 共用 → **一条含 zcode 的 MCP 记录会让全部 MCP 记录读不出来** |
| 静默 `[]` | `repo/rules.rs:41-47` | `canonical_targets(&from_str(...).unwrap_or_default())` |
| 静默 `[]` | `repo/rules.rs:70-75`、`repo/mcp.rs:218-222` | `applied_targets` |
| **静默错值** | `repo/binding.rs:6-12` | `TargetKind::parse(&target_s).unwrap_or(TargetKind::ClaudeCode)` |

`unwrap_or(TargetKind::ClaudeCode)` 的具体危害（确证，按调用点核对）：

- `list_bindings`（`repo/binding.rs:41-51`，**无 ORDER BY**）→ `commands/targets.rs:39` → `:49 bindings.iter().find(|b| b.target == kind)`：拿行序第一个。若用户从未应用过 Claude Code 但应用过 ZCode，**Claude Code 卡片会显示 ZCode 的站点/模型**，`detect_status` 会用 zcode 的 binding 去跑 claude 的判定。
- 同 `bindings` → `tray_apply.rs:393-403`：`has_claude = bindings.iter().any(|b| b.target == ClaudeCode)` 被 zcode 行**误置为 true** → `pick_tray_targets` 加入 ClaudeCode → `:410` `find` 可能取到 zcode 行 → `hydrate_claude_with_binding(&site, status, Some(zcode行))` → 走 claude 应用路径并以 `as_str()="claude_code"` upsert（`repo/binding.rs:53-71`）→ **覆盖真 claude_code 行的快照字段**。
- 同 `bindings` → `local_proxy/mod.rs:54-69`：接管面板显示错误的「当前绑定站点」。
- `list_bindings_for_site`（`repo/binding.rs:109-119`，同样用 `map_binding`）→ `commands/sites.rs:197-234` 删站点带清理：zcode 行被当成 ClaudeCode → 跑 `claude_code::surgical_revert`。该函数**忽略 `managed_paths`**、走 `settings_path(claude_home_override)`（`adapters/claude_code.rs:373-377`），但其 `:397-402` 会按 `key_fingerprint` 匹配并**从 `~/.claude/settings.json` 删除 `ANTHROPIC_API_KEY` / `ANTHROPIC_AUTH_TOKEN`**。ZCode 的 binding 也存了同一个 key 的指纹（`adapters/zcode.rs:100`）→ 同站点同时应用过 Claude 与 ZCode 时**会误删用户的 Claude 凭据**。紧接着 `delete_binding(c, b.target)` 删的是**真 claude_code 行**，而 zcode 行留着并把 `site_id` 置 NULL（FK `ON DELETE SET NULL`）→ 从此成为一条永远伪装 ClaudeCode 的无主行。

**这条 `unwrap_or(ClaudeCode)` 是整个移除里最隐蔽的雷**：它不报错、不 panic，只是把数据算错。

### 2.5 不 panic（确证）

全仓 `TargetKind` 相关无 `unwrap()` / `expect()` / `panic!()` 直接作用于 `parse` 结果；`parse` 返回 `Option`，三个调用点分别用 `unwrap_or`（`binding.rs:12`）、`and_then`（`commands/apply.rs:931`、`local_proxy/routing.rs:140`）。所以**不存在 panic 型故障**，只有 Err 传播型与静默错值型。

---

## 3. 清理能力的丧失（最重要）

### 3.1 `~/.zcode` 实际被写入了哪些痕迹（确证）

路径解析：`paths.rs:569-607`。优先级 = 应用内 `zcode_home_override` → 环境变量 `ZCODE_HOME` → `~/.zcode`（`paths.rs:570-586`，与 Pi/Prime 同构，`ZCODE_HOME` 只在 `default_zcode_home` 里读）。

| # | 落点 | 写入者 | 内容 / 键 |
|---|---|---|---|
| 1 | `~/.zcode/v2/config.json`（`paths.rs:589-593`） | `adapters/zcode.rs:373-443` `apply` | 顶层 `provider` 表下的 **`xiaobai_<id>`** provider 条目，含明文 `apiKey`。id 派生：`provider_id(site_id)`（`zcode.rs:73-76`）= `xiaobai_` + site_id 的字母数字前 16 位；前缀常量 `PROVIDER_PREFIX = "xiaobai_"`（`zcode.rs:25`） |
| 2 | `~/.zcode/v2/provider_config.json`（`paths.rs:596-600`） | 同上，`merge_provider_config`（`zcode.rs:255-319`） | `config.providerConfigRules.providerRules[]`（按 `providerId`）、`config.modelConfigRules.providerModelRules[]`（按 `providerId`）、`config.providerOrder[]`（字符串数组，托管项置尾）。`apiType` 来自 `zcode_api_type` |
| 3 | `~/.zcode/cli/config.json`（`paths.rs:603-607`） | `adapters/mcp.rs:411-466` `apply_to_zcode` | **`mcp.servers.xiaobai_<name>`**（嵌套两层，不是顶层 `mcpServers`，`mcp.rs:410` 注释 + `:427-459` 实现）；`env`/`headers` 明文落地 |
| 4 | `~/.zcode/AGENTS.md`，**或** `~/.zcode/CLAUDE.md`（若目录里已存在用户的 `CLAUDE.md`） | `adapters/agent_rules.rs:219-231` + `agents_or_claude`（`:200-216`） | 托管块 `<!-- xiaobai-switch:begin global-rules --> … :end global-rules -->`（常量 `agent_rules.rs:13-14`），正文含明文约束内容 |
| 5 | `~/.zcode/v2/config.json.lock`、`~/.zcode/v2/provider_config.json.lock`、`~/.zcode/cli/config.json.lock` | `adapters/atomic.rs:87-111` `FileLock`（`<文件名>.lock` **目录**） | 正常路径靠 `Drop` 删除（`atomic.rs:114-118`）；进程被杀会残留空 `.lock` 目录 |
| 6 | `~/.zcode/v2/*.xiaobai-<n>.tmp` | `atomic.rs:16` 原子写临时文件 | 崩溃残留 |
| 7 | `~/.xiaobai-switch/backups/zcode/<ms>/{config.json, provider_config.json, xiaobai-backup.json, .origins.json}` | `backup.rs`（目录名 = `target.as_str()`，`backup.rs:131`、`:158`）；`META_FILE = "xiaobai-backup.json"`（`backup.rs:14`）、`BACKUP_ORIGINS_FILE = ".origins.json"`（`atomic.rs:77`） | 应用前备份 |

**注意 `agents.md` 的 MCP / 全局约束表格只列了 4 个目标，完全没提 ZCode**（`grep -i zcode agents.md` → 无命中）。文档口径已经领先代码，本次移除是**把代码拉齐文档**，不是改协议。

### 3.2 直接删 `zcode.rs` 的后果（结论明确）

**会变成永远清不掉的孤儿，且不止 `~/.zcode`。** 逐条：

| 痕迹 | 删 zcode.rs 后还能清吗 | 原因 |
|---|---|---|
| `~/.zcode/v2/*` 里的 `xiaobai_*` provider | **不能** | 唯一入口是 `zcode::{surgical_revert:444, restore_official:453, cleanup_orphans:622}`，全部经 `remove_provider:475 / remove_all_managed:539`。而 `commands/apply.rs:860`、`commands/targets.rs:261`、`commands/sites.rs:227` 三个派发点都是 exhaustive match 的一臂 —— 变体没了，臂就没了，函数也没了 |
| `~/.zcode/cli/config.json` 的 `mcp.servers.xiaobai_*` | **不能**，且**这个更糟** | ① 写入器 `mcp.rs:411` 被删；② `mcp.rs:280` 的 match 臂随 `TargetKind::ZCode` 消失；③ **`mcp_servers.targets_json` 里的 `"zcode"` 让 `repo/mcp.rs:55` 硬 Err，整个 MCP 列表读不出来** —— 用户连「取消勾选 ZCode 再点应用」这个自救动作都做不到。ZCode 客户端仍会加载这些条目 |
| `~/.zcode/AGENTS.md` / `CLAUDE.md` 的托管块 | **不能** | `agent_rules.rs:219-231` 的 `target_path` 臂没了；更糟的是 `agent_rules.targets_json` 解析失败静默变 `[]`（`repo/rules.rs:44`）→ `commands/rules.rs:68-70` 的 `merged_targets(本次, applied_targets)` 两个集合**都空** → 清理循环零次迭代 → **连 Claude/Codex/Pi/Prime 的托管块也一起清不掉** |
| `~/.xiaobai-switch/backups/zcode/` | **列不出来、剪不了、删不掉** | 见 §3.3 |
| `.lock` 目录 / `.xiaobai-*.tmp` 残留 | 不能（无害） | 只占 inode，ZCode 自己会用同名锁，不会互斥错 |
| `target_bindings` 的 zcode 行 | **不能** | 清理只能靠 `delete_binding(target)` 用字符串匹配 —— 前提是我们还认识 `"zcode"` 这个字符串。不认识时它会**永远伪装成 ClaudeCode**（§2.4），比空文件更糟 |
| `sync_meta` 的两个 `*_applied_targets` 里的 `"zcode"` | 自动失效 | 反序列化成 `[]` 时被整体丢弃，但**副作用是其余目标一起丢**（§2.4），所以它不是无害清理，是有害清理 |
| `site_thinking_presets` 的 zcode 行 | 不能（无害） | 按字符串 WHERE 永不命中，只留指纹噪声 |
| `apply_records` 的 zcode 行 | 不能（基本无害） | `target` 是 `String`，历史列表会显示一个未知目标标签 |

**关键洞察**：ZCode 的「接管未托管同名条目」逻辑其实**不在 zcode.rs 里**，而在共享函数 `mcp.rs:135-145`（`merge_servers_into_json` 的 takeover 循环 + `:148` 的 `retain`）—— agents.md 描述的「先接管、避免重复加载」。所以删 ZCode 不会破坏其它四个目标的接管语义。但**`mcp_scan.rs` 的两个 `_` 兜底臂会静默改变语义**（确证）：

- `mcp_scan.rs:168-172` `json_servers_value`：`ScanTarget::ZCode => root.get("mcp").and_then(|m| m.get("servers"))`，其余 `_ => root.get("mcpServers")`
- `mcp_scan.rs:176-179` `json_servers_key`：同上，`_ => "mcpServers"`

删掉 `ScanTarget::ZCode` 变体后这两处 match **仍然编译**（`_` 兜底在），只是特判臂变成 unreachable code —— 若实现者顺手把 `ScanTarget::ZCode` 从枚举删了却留着别的 `ZCode` 痕迹（例如 `ZCODE_HOME` 派生路径），没有任何编译错误会提醒。**这是唯一一处「删干净反而没有信号」的地方。**

### 3.3 备份侧的现状（确证，且已发布版本就有缺陷）

`parse_backup_id`（`backup.rs:186-202`）对未知前缀 **返回 Err**（`:195-197`），不是忽略。而：

- `prune_all`（`backup.rs:82-88`）**包含** `TargetKind::ZCode`（`:86`）
- `list_backups` 命令（`commands/apply.rs:923-941`）默认目标列表**只有 4 个：ClaudeCode/Codex/Pi/Prime，已经不含 ZCode**（`:935-939`）

我用 `git show v0.1.5:src-tauri/src/commands/apply.rs` 与 `git show v0.1.5:src-tauri/src/backup.rs` 核对过 —— **v0.1.5 发布版就是这个状态**（列表函数在 `:890-906` 同样缺 ZCode，`prune_all` 在 `:80-88` 含 ZCode）。

→ **确证：v0.1.5 的真实用户已经看不到自己的 ZCode 备份**（`backup.rs:124-134` 的 `list_backups_in` 按 `targets` 参数遍历，不在列表里的目录不出现），但 `prune_all` 还在按上限静默删除那个目录。

推论对本次移除的含义：
- 删掉 `prune_all` 里的 ZCode 臂（`backup.rs:86`）**实际上是更安全**的 —— 停止删一个用户在 UI 里看不见、也无法恢复的备份目录。
- 但 `~/.xiaobai-switch/backups/zcode/` 会**永远堆积**。建议：把它列入「保留的最小清理面」，或至少在 release note 里给出手工删除路径。
- `delete_backup(id)` / `restore_backup(id)`（`commands/apply.rs:951-957`）直接吃 `parse_backup_id` 的字符串 —— **只要保留 `parse_backup_id` 的 `zcode-` 前缀识别，即便 UI 不列出，也仍可用一个手工 id 恢复/删除**。这是低成本保住能力的现成口子。
- `mapped_dest`（`backup.rs:259-264`）有 `_ => None` 兜底 → 删 ZCode 臂后，即使 `parse_backup_id` 认得 `zcode-`，`restore_backup_in` 也会因所有文件映射不到目标而报 `"no restorable files in this backup"`（`backup.rs:346-349`）。**两个函数必须同进退。**

### 3.4 最小残留路径：可行且推荐（推测，但设计依据确证）

**确证**：ZCode 的清理面天然与写入口分离 —— `surgical_revert` / `restore_official` / `cleanup_orphans`（`zcode.rs:444/453/622`）以及 `mcp::apply_to_zcode`（`mcp.rs:411`）的**唯一 UI 入口**是三个 exhaustive match 臂（`commands/apply.rs:792/860`、`commands/targets.rs:261`、`commands/sites.rs:227`、`commands/mcp.rs:302`）。撤掉入口而不删函数，编译不受影响。

**推测（推荐方案，两阶段）**：

阶段一（发一个过渡版本，风险最低）：
1. **保留** `TargetKind::ZCode` 变体、`as_str`、`parse("zcode")`、`paths::zcode_*` 五个函数（`paths.rs:569-607`）。
2. 从**所有目标清单数组**里移除 ZCode，使其不可勾选/不可枚举：`commands/targets.rs:42-48`、`local_proxy/mod.rs:59-65`、`adapters/agent_rules.rs:243-250`、`adapters/mcp_scan.rs:346-352`、`backup.rs:82-88`、`commands/mcp.rs:355-376`、`tray_apply.rs:335-365`。→ 主界面/托盘/接管面板/应用中心都不再出现 ZCode。
3. **加一次性启动清洗**（必须在**任何 typed read 之前**跑，且用裸字符串/`serde_json::Value` 操作，不能先 `get_settings`）：
   - `settings` blob：解析成 `Value`，从 `localProxyTargets` 数组里剔掉 `"zcode"`，删掉 `zcodeHomeOverride` 键，回写。
   - `mcp_servers.targets_json` / `agent_rules.targets_json` / `sync_meta` 两个 key：同法剔元素。
   - `DELETE FROM target_bindings WHERE target='zcode'`（**先**调 `zcode::cleanup_orphans(override)` 清盘，再删行）。
   - `DELETE FROM site_thinking_presets WHERE target='zcode'`。
   - `apply_records` 保留（历史，`target` 是 String）。
4. **清理能力靠一条隐藏命令兜底**：保留一个 `cleanup_zcode_leftovers` command（或复用阶段一尚未删的 `unapply_target`），只允许「清理」不允许「应用」；或在清洗器里直接调用 `zcode::remove_all_managed` + `mcp` 的 ZCode 分支。UI 不需要入口。

阶段二（后续版本）：清洗器跑过 N 个版本、WebDAV 对端也都升级后，再删 `TargetKind::ZCode` 变体、`zcode.rs`、`zcode_api_type` 列相关代码。
**但 `sites.zcode_api_type` 列建议永久保留不 DROP**（理由见 §2.3 指纹）。可以只删 SELECT 项 + DTO 字段 + 读写代码。

验证清洗器有效性的方法（推测）：用 `sqlite3` 手工构造一个 `settings.json` 含 `"localProxyTargets":["claude_code","zcode"]` 的库 + `mcp_servers.targets_json='["zcode"]'` + `agent_rules.targets_json='["zcode"]'`，跑清洗后再跑 `get_settings` / `repo::mcp::list` / `repo::rules::get`，断言三者皆 Ok 且不含 zcode 痕迹。

---


---

> 本文件的后半部分（自 `## 4.` 起）已拆分到 [`backend-target-surface-and-redlines.md`](./backend-target-surface-and-redlines.md)，以免超出 context injection 的 32768 字节上限被截断。两份必须一起读。
