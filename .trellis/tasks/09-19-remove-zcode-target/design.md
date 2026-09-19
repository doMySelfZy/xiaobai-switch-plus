# 设计：移除 ZCode 应用目标

配套 `prd.md`。需求编号 R1–R9、验收编号 AC1–AC11 均指向该文件。

## 1. 问题本质

移除一个**已发布**的应用目标，难点不在删代码，而在三条既成事实：

1. `TargetKind` 被序列化进了多处持久化 JSON，删枚举变体等于让旧数据变成非法输入；
2. 本应用已经把 `xiaobai_*` 条目写进了用户机器上的 `~/.zcode/`，删掉写入器同时也删掉了清理器；
3. 逻辑指纹对 `sites` 表走 `SELECT *` 逐列哈希，改列集会污染跨机同步判定。

设计目标因此是：**产品面归零，协议面与清理面按需保留最小残留，且残留必须是「迁移代码」而非「功能代码」。**

## 2. 架构与边界

改动分四个内聚单元，彼此有严格的先后依赖：

```
U1 数据撤退层   src-tauri/src/zcode_retirement.rs   （新增，唯一允许含 zcode 字面量的生产模块）
      │  ├─ DB 侧：ensure_in_db(conn)      —— 幂等，由 ensure_incremental_schema 调用
      │  └─ 文件侧：clean_external_once()  —— 幂等，由 lib.rs setup 调用一次
U2 目标模型收缩  src-tauri/src/domain/mod.rs 等     —— 删变体，穷举 match 由编译器逼出
      │
U3 后端消费面    commands/ tray/ adapters/ local_proxy/ *_probe/ deep_link
      │
U4 前端与契约    types/ stores/ components/ pages/ i18n/ browserMock/ tests
```

**依赖方向是 U1 → U2 → U3 → U4。** U1 必须先落地：只有清洗器在位，U2 删变体才不会让库变成读不出来；U3/U4 删穷举分支后编译器才会报出全部遗漏点。反过来先删枚举，会在中间态留下一个「打不开自己数据库」的构建。

### U1 的模块边界（关键取舍）

`zcode_retirement.rs` 是**自包含**的：它只认识 `"zcode"` 这个字符串字面量、`xiaobai_` 前缀、`<!-- xiaobai-switch:begin global-rules -->` 标记，以及 `~/.zcode` 下三处落点的相对路径。它**不 import `adapters::zcode`**（该模块在 U3 被删），也不 import `TargetKind::ZCode`（该变体在 U2 被删）。

这是「彻底删干净」与「不留孤儿」能同时成立的唯一结构：功能代码归零，残留的是一个会随下一次大版本一起删掉的撤退脚本。文件头以注释写明它为什么存在、以及**什么条件下可以整文件删除**（存量 zcode 数据自然消化完毕之后）。

## 3. 数据流与契约

### 3.1 DB 撤退清洗（`ensure_in_db`）

挂载点：`ensure_incremental_schema`（`db/migrate.rs:280`）。已验证 `apply_schema` 的三个分支（`:297` 版本达标 / `:312` 旧库迁移 / `:321` 全新库）**全部**经过它，因此首启、升级、`app_backup.rs:325` 恢复、`pending_restore.rs` 恢复四条路径都被覆盖 —— 这正是 `agents.md` 里「`CREATE TABLE IF NOT EXISTS` 不会补列，每个库已存在分支都要跑增量补齐」那条约束的用处所在。

**不递增 `SCHEMA_VERSION`**：本次不改任何表结构，清洗是数据外科手术。保持版本不变，新旧客户端对同一库的结构认知一致。

处理清单与手段：

| 目标 | 位置 | 手段 |
|---|---|---|
| 设置 blob 里的代理目标数组（实际落库键名是驼峰 `localProxyTargets`） | `settings` 表 `id=1` | 见下 |
| `targets_json` 数组含退役目标 | `mcp_servers`、`agent_rules` | 见下 |
| applied targets 数组含退役目标 | `sync_meta`，键 `agent_rules_applied_targets`（`repo/rules.rs:8`）**与 `mcp_applied_targets`（`repo/mcp.rs:11`）** | 见下 |
| `target` 列为退役目标的行 | `target_bindings`、`apply_records` | `DELETE WHERE target = ?1 OR target = ?2`（两种拼写） |
| 残留值 | `sites.zcode_api_type` | `UPDATE ... SET zcode_api_type = NULL`（**列保留**） |

**⚠ 执行阶段实证的关键事实：库里同时存在两种拼写，清洗必须双识别。**

`TargetKind` 有两条互不相同的持久化通道，本任务最初的夹具口径（`prd.md` / `implement.md` 0.1 写的 `"zcode"`）只对了一半：

- `serde` 通道（`settings` blob、`mcp_servers.targets_json`、`agent_rules.targets_json`、`sync_meta.*`）：`#[serde(rename_all = "snake_case")]` 对 `ZCode` 产出 **`"z_code"`**。serde_derive 的 `SnakeCase` 实现（`serde_derive-1.0.229/src/internals/case.rs:63-72`）在**每个** `i > 0` 的大写字母前插 `_`，`ZCode` = `Z` + `_C` + `ode` → `z_code`。
- `as_str()` 通道（`target_bindings.target`、`apply_records.target`）：`domain/mod.rs:91` 手写 **`"zcode"`**，`TargetKind::parse`（`:100`）也只认 `"zcode"`。

也就是说 v0.1.5 时代**前端往 MCP / 全局约束的目标数组里塞 `"zcode"` 根本反序列化不回来**（serde 期望 `z_code`），两条通道一直是分裂的。撤退层因此按「任一层字符串数组里出现 `z_code` 或 `zcode`」识别，不认具体键名 —— 键名会随重构漂移，值不会。这条同时是 U2 的前置知识：删变体时不要天真地只 grep `"zcode"`。

数组清洗手段：`SELECT` 原文 → 仅当字符串里**确实包含**带引号的退役名（`"z_code"` / `"zcode"`）时才 `serde_json::from_str::<Value>` → 递归剔除数组元素里的退役名 → 回写。不含则跳过，不产生任何写。自由文本里提到 `zcode`（例如 `sync_meta` 某条值写着「zcode 用户指南」）不算脏数据，必须有 `zcode_retirement::free_text_mentioning_the_name_does_not_trigger_a_write` 这条守住。


**已验证的副作用**：`serde_json = "1"` 未启用 `preserve_order`（`src-tauri/Cargo.toml:32`），`Map` 即 `BTreeMap`，因此往返会把对象键重排为字母序。只影响被清洗的那一行、且只影响键顺序不影响语义；下次正常 `save_settings` 会按结构体声明序重写。设计上接受它，换取「不做字符串手工切割」的健壮性。

若某行 JSON 已损坏到无法解析，**保留原值不报错**（本次清洗不是数据修复入口），但必须在 `tracing::warn!` 里记录表名与主键，避免又变成一处静默。

### 3.2 文件侧撤退清理（`clean_external_once`）

调用点在 `lib.rs` 的 `setup`（`:69`），紧接数据库就绪之后、托盘与代理初始化之前。**不放 `apply_schema` 里**：那条路径也会被备份恢复与外部库校验复用（`app_backup.rs:325`、`pending_restore.rs:496/615`），在那些场合动用户真实 `~/.zcode` 是错的。

落点（路径解析沿用 `src-tauri/src/paths.rs` 的现有约定，注意应用数据根是 `~/.xiaobai-switch/`，目标是用户 CLI 目录）：

1. `~/.zcode/v2/config.json` —— 剔除 `provider.xiaobai_*`（写入方 `adapters/zcode.rs:141`）；
2. `~/.zcode/v2/provider_config.json` —— 在 `config` 下剔除三处：`providerOrder[]` 里以 `xiaobai_` 开头的项、`providerConfigRules.providerRules[]` 与 `modelConfigRules.providerModelRules[]` 里 `providerId` 以 `xiaobai_` 开头的项（写入方 `adapters/zcode.rs:251-361`）；
3. `~/.zcode/cli/config.json` —— 剔除 `mcp.servers.xiaobai_*`。注意**不是顶层 `mcpServers`**，ZCode 用的是 `mcp.servers`（`adapters/mcp.rs:409` 起）；
4. `~/.zcode/AGENTS.md`、`~/.zcode/CLAUDE.md` —— 摘掉 `<!-- xiaobai-switch:begin global-rules -->` … `end` 托管块。二者都要扫：写入方 `adapters/agent_rules.rs:228` 走 `agents_or_claude()`，用户只有 `CLAUDE.md` 时是追加进那个文件，不会新建 `AGENTS.md`。

契约沿用仓库既有写盘规范：写前备份到 `~/.xiaobai-switch/backups/`，`atomic.rs` 的 `.xiaobai-` 临时文件原子替换，与目标 CLI 各自的原生位置互斥（`<文件名>.lock`）。**块外用户内容逐字节保留（含 UTF-8 BOM）**，唯一允许的字节变化仍是尾部空行归一。

清理规则（对齐 `agents.md` 既有的「不合法就报错并原样保留」口径）：

- 目录/文件不存在 → 静默跳过，不创建、不报错、不记 warn；
- 文件非 UTF-8、是目录、`mcp.servers` 不是对象、只有 BEGIN 没有 END、正文含标记字面量 → **跳过该文件并 warn，绝不覆盖**；
- 文件清理后只剩空白（整份都是本应用写的）→ 删除该文件（与 `agents.md` 全局约束的既有规则一致，否则空 `AGENTS.md` 会反过来遮蔽用户自己的 `CLAUDE.md`）；
- 无 `xiaobai_*`、无托管块 → 不备份、不写盘（内容无变化不产生痕迹）。

频率：每次启动都跑，靠幂等保证安全。刻意**不加 `sync_meta` 完成标记**，理由是标记会被 WebDAV 同步带到别的机器上，反而可能让那台机器的 `~/.zcode` 永远清不掉；纯幂等重跑的代价只是几次 stat，可接受。

### 3.3 顺带修掉的两处静默放大器

这两处不是顺手重构，而是本次「删变体」动作直接暴露出来的事故放大器，属于 R2 前置：

- `repo/binding.rs:12` 的 `TargetKind::parse(&target_s).unwrap_or(TargetKind::ClaudeCode)`。`parse` 已返回 `Option`（`domain/mod.rs:95-105`），这里吞掉 `None` 并冒充 Claude Code 才是误删用户鉴权键的根因。改为让 `map_binding` 返回 `AppResult`（或该行走 `skip`），未知目标行**不进列表**，并由 `ensure_in_db` 的 `DELETE` 负责最终清除。
- `repo/rules.rs:44` 与 `:72` 的 `unwrap_or_default()`。解析失败与「本来就没勾目标」被压成同一个空数组，于是任何一次脏数据都会静默清空 4 个目标的清理集合。改为解析失败时 `tracing::warn!` 并保留可区分的错误路径。

## 4. 兼容性与红线（R4 的展开）

不动清单，逐条理由：

- **`FINGERPRINT_TABLES` 与 `FINGERPRINT_ALGORITHM_VERSION` 不改**（`sync.rs:41-51`、`:56`）。`sync.rs:53-62` 记录的真实事故就是表清单 8→9 漂移导致 0.1.3/0.1.4 两端对同一数据算出恒定不同指纹、反复互相覆盖。
- **`sites.zcode_api_type` 物理列不 DROP**。`compute_logical_fingerprint`（`sync.rs:81`）是 `SELECT * FROM {table} ORDER BY rowid` 后逐列哈希；删列会让**所有**既有库的 `sites` 指纹改变，而 v0.1.5 对端仍带着该列 → 跨机指纹永久不等 → 同类循环复现。SQLite 删列还需整表重建，风险与收益完全不成比例。代码侧只从 `SITE_SELECT`（`repo/site.rs:89`）与 `row.get(23)`（`:52`）、`INSERT` 列清单（`:187`）、`UPDATE`（`:327`）移除该字段，结构体字段一并删除。
- **`xiaobai_` 命名空间、MCP 服务器名派生规则、托管块标记字面量、备份文件名前缀双识别（`xiaobai-switch-backup-` 与 `any-switch-backup-`）、备份包内条目名 `xiaobai-switch.db`、WebDAV manifest 名与远端目录名、深链旧 scheme、更新签名公钥**：全部保持原值。U1 恰恰依赖这些字面量不变才能认出并清掉自己写过的东西。
- **`restore_official` / 孤儿清理识别逻辑不削弱**（`agents.md` 明文红线）。这是保留 U1 文件侧清理器的直接依据。

## 5. 主要取舍

| 取舍 | 选择 | 放弃的方案与代价 |
|---|---|---|
| 是否保留 `TargetKind::ZCode` 作只读兼容载体 | **不保留**，改为升级前幂等清洗 | 保留载体最省事且零风险，但用户已明确要「彻底删干净」，代码里会永久留着死变体与死分支 |
| 数组清洗位置：读路径容错 vs 一次性写库清洗 | **一次性写库清洗** | 读路径容错能让未升级的对端也不炸，但会在每个 repo 层留长期分支，且无法清掉 `target_bindings` 行 |
| `~/.zcode` 外部清理：留撤退脚本 vs 不管孤儿 | **留脚本** | 不管孤儿违反 `agents.md` 孤儿清理红线；代价是生产代码里仍有含 zcode 字面量的文件（AC10 因此允许该例外） |
| 外部清理触发：每次幂等 vs 一次性标记 | **每次幂等** | 标记会被同步污染其他机器；代价是每次启动多几次 stat |
| 键序重排 | 接受 | 手工字符串手术可保序但脆弱，不值得 |
| 分支 | 当前 `feat/site-add-presets`（用户定） | 独立分支可单独回滚；代价是与 M1/M2 在 5 个文件上是冲突热区 |

## 6. 回滚与运维

- **代码回滚**：单分支，`git revert` 本次提交即可。
- **数据回滚不成立**：`ensure_in_db` 的 `DELETE target='zcode'` 与 `UPDATE zcode_api_type = NULL` 是有损的。缓解措施是**清洗前先备份整个库**到 `~/.xiaobai-switch/backups/`（复用 `backup_pre_migration`，`migrate.rs:690`），且只在真正发生写变更时备份一次。设计接受这一不可逆性：这些行的语义就是「指向一个即将不存在的目标」，留着只会继续触发 §3.3 的误删路径。
- **跨机不一致窗口**：A 机已升级并清洗，B 机仍 v0.1.5。B 机会把自己那份含 zcode 的 `settings` blob 推上去。因表清单未变、算法版本未变，判定仍按内容新旧走，A 机再启动会重新清洗（幂等）。**已知可接受**，但必须在 PR 说明里写清：本次改动后，对端应尽快一起升级。
- **发布提示**：release note 需说明 ZCode 目标已移除、`~/.zcode` 下由本应用写入的条目会被自动清理一次、用户自己的 ZCode 配置不动。

## 7. 验证策略

顺序上先建立「**存量夹具测试红灯**」再动手：用一个 v0.1.5 形状的真实数据库 fixture 断言 §3.1 全部行为，此时它必红；实现 U1 后转绿；此后 U2–U4 每步都要它保持绿。这是 AC1–AC4 的守门测试，也是唯一能在删穷举分支时兜住静默语义变化的机制（`mcp_scan.rs` 的 `ScanTarget` 带 `_` 兜底臂，编译器不会报错）。

前端以 `pnpm typecheck` 作穷举结构的探雷器（3 个 `Record<TargetKind, string>` + `MENU_ICONS` 会精确报错），但它**照不到** `TargetStatusCard.tsx:302` 默认臂与 `hydrateApplyForm.ts:110-139` 的 `export` 死代码 —— 那两处只能靠 `research/frontend-removal-worklist.md` 的移除清单逐条核。

命令见 `implement.md`。
