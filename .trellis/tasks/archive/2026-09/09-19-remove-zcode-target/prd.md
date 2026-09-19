# 移除 ZCode 应用目标支持

## Goal

把 ZCode 从 XiaoBaiSwitch Plus 的应用目标中彻底移除，回到 **Claude Code / Codex / Pi / Prime** 四目标形态；并且不能让已经拿到 v0.1.5 的用户在升级后出现配置读不出、存不了、或本应用误删其真实 CLI 配置的事故。

判断「移除成功」的标准不是「grep 不到 zcode」，而是：**代码与 UI 里不再有 ZCode 这个可操作目标，同时存量数据与存量落盘痕迹都被安全收口。**

## Background / Confirmed facts

以下均已实读源码验证（非推测）：

- **ZCode 已发布给真实用户。** 提交 `21782f0`（2026-09-15，`feat(zcode): 后端支持 ZCode 作为第五个应用目标`）已在 `main`，且包含在 tag `v0.1.5` 内。
- **本仓库文档口径已经是 4 目标。** `agents.md` 与 `.trellis/spec/` 全文零 `zcode` 命中。本次是把代码拉齐既有文档，不新增兼容红线冲突。
- **`TargetKind` 是裸 serde 枚举**（`src-tauri/src/domain/mod.rs:74-82`，`#[serde(rename_all = "snake_case")]`，`ZCode` 为第 5 个变体，无 `#[serde(other)]`）。删掉该变体后，持久化 JSON 里残留的 `"zcode"` 是**未知变体 → 整份反序列化失败**。同文件已有 `#[serde(alias = "prime_agent", alias = "prime-agent")]` 的协议兼容先例。
- **设置整份存成单个 JSON blob。** `src-tauri/src/repo/settings.rs:37` 用 `?` 硬失败；`AppSettings.local_proxy_targets: Vec<TargetKind>` 在 `domain/mod.rs:614`。→ 勾选过 ZCode 的 v0.1.5 用户升级后设置页**既读不出也存不了**，本地代理与开机自启静默失效（`lib.rs:99-107`、`autostart.rs:80-85` 各自吞错），而客户端 Base URL 仍指向代理。
- **一条脏行使整个 MCP 列表不可读。** `src-tauri/src/repo/mcp.rs:55` `serde_json::from_str(&targets_json)?`。
- **全局约束目标解析失败会静默变空数组。** `src-tauri/src/repo/rules.rs:44` 与 `:72` 均为 `unwrap_or_default()`。→ `merged_targets` 两个集合都空，**Claude/Codex/Pi/Prime 的托管块也一起清不掉**。
- **残留 zcode 绑定行会伪装成 Claude Code。** `src-tauri/src/repo/binding.rs:12` `TargetKind::parse(&target_s).unwrap_or(TargetKind::ClaudeCode)`（`parse` 本身返回 `Option`，见 `domain/mod.rs:95-105`）。→ 托盘 `has_claude` 误置（`tray_apply.rs:397`），删站点时跑 `claude_code::surgical_revert`，其按 `key_fingerprint` 匹配（`claude_code.rs:397-402`）可能**误删用户 `~/.claude/settings.json` 里的鉴权 env 键**。
- **逻辑指纹走 `SELECT *`。** `src-tauri/src/sync.rs:81` `SELECT * FROM {table} ORDER BY rowid` 后逐列哈希；`FINGERPRINT_TABLES` 为 9 张表（`sync.rs:41-51`），含 `settings`/`sites`/`target_bindings`/`apply_records`/`mcp_servers`/`agent_rules`。`sync.rs:53-62` 明文记录过 0.1.3（8 表）与 0.1.4+（9 表）算法不一致导致的**跨机互相覆盖循环**。
- **`sites.zcode_api_type` 是位置列。** 建表 `src-tauri/src/db/migrate.rs:61`，增量补列 `:209-210`，读取 `src-tauri/src/repo/site.rs:52` 用 `row.get(23)`，`SITE_SELECT` 显式列在 `repo/site.rs:89`。
- **`db/migrate.rs:289 apply_schema` 是唯一 schema 入口**，并被备份/恢复路径共用（`app_backup.rs:325`、`db/mod.rs:34/46`、`deep_link.rs:220`、`pending_restore.rs:496/615`）。
- **前端存在三份并列目标联合**：`TargetKind`（`src/types/domain.ts:15`）、`ApplyTargetTab`（`src/stores/uiStore.ts:7`）、`ScanTarget`（`src/types/mcp.ts:103`）。穷举压力来自 3 个 `Record<TargetKind, string>`（`McpPage.tsx:54`、`ProxyPage.tsx:12`、`RulesPage.tsx:12`）与 `MENU_ICONS`（`ApplySidebar.tsx:15`）。
- **前端无持久化脏键。** 全仓零 zustand `persist`、零 target 相关 `localStorage`；ZCode 存量只存在于 SQLite。
- **`hydrateApplyForm.ts:110-139` 的 zcode 块全部是 `export`** → 删面板后变静默死代码，`noUnusedLocals` 抓不到。
- **`TargetStatusCard.tsx:302` 的默认兜底臂恰好返回 ZCode 标签** → 删类型成员后此处不报错，却会把未知/遗留 kind 一律标成「ZCode」。
- **`ProxyPage.tsx:168/169/360` 索引 `TARGET_LABEL_KEYS[target]` 无 `?? target` 兜底**（另两个页面有）→ 后端若仍回 zcode，`t(undefined)` 有崩溃风险。
- 61 个源文件命中 zcode。详细清单见 `research/frontend-surface.md` 与 `research/backend-data-compat.md`。
- **仓库根 `.zcode/` 与本次无关。** 那是 Trellis 给 ZCode CLI 用的平台配置目录（本次会话的钩子另有其源），**禁止删除**。产品目标是用户机器上的 `~/.zcode/`。

## Requirements

### R1 后端：目标与适配器彻底移除

删除 `TargetKind::ZCode` 变体、`src-tauri/src/adapters/zcode.rs`（1005 行）、`adapters/mod.rs` 的 `mod zcode;`，以及 `commands/targets.rs:41-47` 的 5 元遍历、`tray.rs`(32)、`tray_apply.rs`(22)、`route_switch.rs`、`key_switch.rs`、`local_proxy/{mod,routing,server}.rs`、`quota_probe/mod.rs`、`models_fetch/mod.rs`、`deep_link.rs`、`commands/{apply,mcp,sites,settings,rules}.rs` 里的 ZCode 分支。

同时删除 `adapters/mcp_scan.rs` 的 `ScanTarget::ZCode`（`:32` 附近，另注意它的两个 `match` 带 `_` 兜底 → 删变体后**编译通过但语义静默改变**，必须逐个复核而非依赖编译器）。`commands/skills.rs:42` 的 `SkillTarget` 本来就不含 ZCode，**不得改动**。

### R2 后端：升级前必须完成的一次性幂等清洗

在 `db/migrate.rs` 的既有增量补齐路径（`ensure_incremental_schema`，`:280`）里加入 **ZCode 撤退清洗**，且必须在任何业务读取之前生效。要求**幂等**（第二次运行是 no-op），从而恢复旧备份时也能再洗一遍。覆盖：

- `settings` 表 JSON blob 内 `local_proxy_targets` 数组中的 `"zcode"`；
- `mcp_servers.targets_json`、`agent_rules.targets_json`、`sync_meta` 的 `agent_rules_applied_targets`（`repo/rules.rs:8`）中值为 `"zcode"` 的数组元素；
- `target_bindings` 与 `apply_records` 中 `target = 'zcode'` 的整行 —— **删除，不得重映射成 `claude_code`**；
- `sites.zcode_api_type` 的值置 `NULL`（**置空，不 DROP 列**）。

顺带修掉两处静默错误放大器（属本次移除的必要前置，不是顺手重构）：
`repo/binding.rs:12` 的 `unwrap_or(TargetKind::ClaudeCode)` 改为「未知目标 → 判定该绑定不可用并跳过」；`repo/rules.rs:44/72` 的 `unwrap_or_default()` 至少需区分「解析失败」与「本来就为空」，避免未来任何一次脏数据把 4 个目标的清理能力一起静默清零。

### R3 后端：`~/.zcode` 落盘痕迹的一次性撤退清理

v0.1.5 可能已在用户机器 `~/.zcode` 下写入（解析见 `src-tauri/src/paths.rs`）：

- `v2/config.json` 与 `v2/provider_config.json` 里的 `xiaobai_*` 条目；
- `cli/config.json` 的 `mcp.servers.xiaobai_*`；
- `AGENTS.md` / `CLAUDE.md` 里 `<!-- xiaobai-switch:begin global-rules -->` 托管块。

必须提供一个**自包含、不依赖 `adapters/zcode.rs`** 的一次性撤退清理器（含写前备份与原子替换，遵循既有 `atomic.rs` 与互斥锁约定），跑完后不再提供 ZCode 写入口。理由：`agents.md` 兼容红线明文要求「`restore_official` / 孤儿清理的识别逻辑不得改动」，让本应用自己写进去的东西变成永久孤儿违反该红线。清理器属迁移代码，不属于「ZCode 支持」。

若 `~/.zcode` 不存在或无相关条目，静默跳过，不报错、不创建文件。

### R4 兼容红线：以下绝对不动

- **不 DROP `sites.zcode_api_type` 物理列**，不改 `FINGERPRINT_TABLES` 表清单，不改 `FINGERPRINT_ALGORITHM_VERSION`（`sync.rs:56`）。改列集会重演 `sync.rs:53-62` 记录的跨机覆盖事故；SQLite 删列还需整表重建。
- 不改 `xiaobai_` 命名空间、MCP 服务器名规则、托管块标记字面量、备份条目名与文件名前缀。
- 不改更新签名公钥。
- `restore_official` 与孤儿清理的识别逻辑不得削弱。

### R5 前端：可操作面全删

删除 `src/components/apply/ZCodeApplyPanel.tsx`(237 行) 及其测试、`ApplyPage.tsx:100-112` 的第 5 段手工 JSX、`hydrateApplyForm.ts:110-139` 的导出块、`ApplySidebar.tsx` 的 `MENU_ICONS` 条目（删后 `:5` 的 lucide `SquareTerminal` import 变未使用 → `noUnusedLocals` 报错，需一并删）、`ApplyFooter.tsx`、`TargetStatusCard.tsx`、`showApplyOutcome.tsx`、`SettingsPage.tsx:477/484/490/499 + 533-541`（ZCode 路径覆盖输入框）、`McpPage.tsx`、`ProxyPage.tsx`、`RulesPage.tsx` 的目标数组与标签映射、`stores/{apply,ui,settings}.ts`、`types/domain.ts`、`types/mcp.ts`。

必须主动改写两处编译器抓不到的点：`TargetStatusCard.tsx:302` 的默认臂（当前恰好是 ZCode 标签）、`ProxyPage.tsx:168/169/360` 缺失的 `?? target` 兜底。

三份并列联合（`TargetKind` / `ApplyTargetTab` / `ScanTarget`）与 `McpPage.tsx:1107` 的 `entry.target as TargetKind` 隐式不变式必须**同批**修改。

### R6 前端：i18n 中英双侧对称删除

`zh-CN.json` 与 `en-US.json` 各 22 个 zcode 键、键集合完全对称，两文件各 894 键。除按名删除外，必须处理**键名不含 zcode 但文案提到它**的复用键 `apply.dualWarning`；`sites.goApplyZCode` 是死键，其同族 `sites.goApply{Claude,Codex,Pi,Prime}` 同样全死，一并评估。按名过滤必然漏，须以文案正文再扫一遍。

### R7 前端：mock 契约与 Rust command 同步

`src/lib/browserMock.ts`(25 命中) 约 12 个位点须与真实 command 同批删除（含 `apply.rs:64` 的 `zcode_write_all_models`、`commands/targets.rs:41-47`、`mcp_scan.rs:32`）。注意 `preview_merge` 无 `deny_unknown_fields` → 多传字段被静默忽略，契约漂移不会报警，须人工核对而非依赖运行时报错。

### R8 测试

前端 6 个测试文件需处理（1 个删文件、5 个改行），其中 `McpPage.test.tsx:471` 的 `toHaveLength(3)` 含 ZCode 项 → 必须改为 2；`mockProxyStatus` 的 targets 顺序敏感。Rust 侧删 ZCode 断言，并**新增回归测试**：含 `"zcode"` 的存量 JSON 与 `target='zcode'` 行经清洗后，设置可读可存、MCP 列表可读、4 个目标的托管块仍可清理。

`packagingWorkflow.ts` 里的 "targets" 指 Rust target triple，是假阳性，**勿动**。

### R9 文档口径

`README*.md:193` 提到 ZCode。更新为 4 目标；`agents.md` 已是 4 目标口径，无需改（改动前须复核，避免误删既有红线段落）。

## Acceptance Criteria

> 判定口径：勾 = 有可复跑的证据。仅代码/测试级证据的地方显式写明，不当成真机验证。

- [x] **AC1 存量不炸**：取 v0.1.5 真实形状数据库夹具（设置 blob 含 `localProxyTargets:["z_code",...]`、`mcp_servers.targets_json` 含 `"z_code"`、`agent_rules.targets_json` 含 `"z_code"`、`sync_meta` 的 `agent_rules_applied_targets` 与 `mcp_applied_targets` 含 `"z_code"`、`target_bindings` 与 `apply_records` 各至少 1 行退役目标、`sites.zcode_api_type` 非空），启动后设置页可读可保存、MCP 列表可读出、全局约束页可读写。
      > 执行阶段实证修正：JSON 数组站点真实落库拼写是 `"z_code"`（serde `snake_case` 通道），行表列值是 `"zcode"`（`as_str()` 通道）。两种拼写都必须造进夹具、都必须被清掉，依据见 design.md §3.1。
      > 证据：`cargo test zcode_retirement` 27 例全绿（读+存两侧都断言）。真机 GUI 未点，见 implement.md 5.5。
- [x] **AC2 不被伪装**：上述夹具中 `target='zcode'` 的绑定不会以 `claude_code` 身份出现（不被误标为已应用、不进入 claude 回滚路径）。
      > 证据：`unknown_target_binding_is_skipped_instead_of_masquerading_as_claude_code`，且该用例经变异验证（把 `map_binding` 改回 `unwrap_or(ClaudeCode)` 会红）。
- [x] **AC3 其他目标清理能力未退化**：在含脏 zcode 数据的库上，Claude/Codex/Pi/Prime 的托管块与 MCP 托管条目仍可被正常清理；对比清洗前后的 `merged_targets` 集合非空。
      > 证据：`managed_provider_entries_are_pruned_and_user_entries_survive` + `parse_persisted_targets` 的逐元素用例；变异验证覆盖「前缀判断去掉」「改成 no-op」两种破坏。
- [x] **AC4 幂等**：撤退清洗连续执行两次，第二次零行变更；从旧备份恢复后再启动仍能正确清洗。
      > 证据：`retirement_is_idempotent_and_backs_up_exactly_once`（备份挪到写盘之后会红）。
- [x] **AC5 孤儿收口**：构造含 `xiaobai_*` 条目与托管块的假 `~/.zcode`，升级启动后条目与托管块被移除、块外用户内容逐字节保留、写前有备份；目录不存在时全程静默无报错。
      > 追加实现：v0.1.5 的 `zcodeHomeOverride` 虽已从 `AppSettings` 删除，但仍留在设置 blob 文本里；`leftover_home_override` 直接读原始 JSON 取回它，使**自定义目录**用户的明文 Key 残留也能被清掉（3 条新用例）。
- [x] **AC6 同步未被破坏**：`FINGERPRINT_TABLES`、`FINGERPRINT_ALGORITHM_VERSION` 与 `sites` 列集合均无变化；同一份数据在改动前后计算出的指纹差异**只**来自真实业务值变化。
      > 证据：`sync.rs` 本任务零改动；`migrate.rs:61` 建表列与 `:209-210` 增量 `ALTER` 均在（**物理列集合不变**才是指纹的输入，指纹走 `SELECT *`）；`repo/site.rs` 的 `SITE_SELECT` 不再**读**该列（23 列，`row.get` 位序同步收缩），写入侧同样不再带它。
- [x] **AC7 UI 无残留**：应用中心目标侧栏、托盘子菜单、MCP / 代理 / 全局约束 / 设置四处目标勾选均不再出现 ZCode；托盘子菜单顺序无错位。
      > 证据级别：**代码 + 单元/组件测试**（`TAB_KEYS` 4 项、四处 `TARGETS` 数组 4 项、`MENU_ICONS` 无 zcode、托盘 tooltip 断言 `!contains("ZCode")` 且行数 5）**+ 真机渲染走查**：打包版已安装启动，且用 `pnpm dev` 在真浏览器里逐页扫渲染后的 `outerHTML`（六页 + 设置五节）→ `zcode` **零命中**，每页目标复选框集合实测恰为 Claude Code / Codex / Pi / Prime，含 MCP「手动添加」弹窗、全局约束「生效目标」、本地代理、设置「路径」覆盖。**唯一未取得的仍是托盘子菜单的肉眼证据**（原生菜单，浏览器覆盖不到，本机 computer-use 点击落不进 WebView2）—— 该项由 `build_menu` 逐目标硬写 + `TraySnapshot` 字段已删（编译期保证）+ `tray.rs:747` 断言支撑，别当看过界面。
- [x] **AC8 文案无残留**：中英双侧 zcode 键删除后无裸 key 渲染；两文件键集合仍对称。
      > 措辞修订（执行阶段决策，用户「彻底删干净」口径）：`apply.dualWarning` **不改为去掉 ZCode 提法，而是连同 `apply.dualWarningTitle` 一起删除** —— 两条都是全仓零引用的死键（`sites.goApply*` 同族里只有 `sites.goApply` 被 `GoApplyButton.tsx` 使用），给死键改文案不产生用户可见结果。同族其余死键不属本任务，保持原样。
      > 证据：脚本化删除前后键集只差 24 项、中英对称；`grep zcode src/i18n/locales/` 零命中；`pnpm test:run` 432 例无裸 key 断言失败。
- [x] **AC9 检查全绿**：`pnpm typecheck` 零错误；`pnpm test:run` 432 passed（2 个文件收集失败为**既有问题**：`generateUpdaterManifest.test.ts` / `validateUpdaterSigningSecret.test.ts` 顶层 `await import` 报 `SyntaxError`，四个相关文件本任务零改动）；`cargo test` 579 passed / 0 failed / 2 ignored。
      > 已知限制（**收尾时已解除**）：写作此条时本机未装 `clippy` 与 `rustfmt`，「零新增警告」只由 `cargo check --lib --tests` 近似保证。后经用户批准补装两个组件并按增量交集口径核完（clippy 新增行 0 命中、rustfmt 实测证明不是本仓约定），口径与过程见 `implement.md` 阶段 5 的「工具链核查」。
- [x] **AC10 目标计数归零**：`src/` 与 `src-tauri/src/` 的 `zcode` 命中只剩一次性撤退清理器、其挂载点、保留的 DDL 与守门断言；`adapters/zcode.rs` 与 `paths.rs` 的 5 个 ZCode 函数均已删除，无任何写入型 ZCode 代码路径。
- [x] **AC11 仓库根 `.zcode/` 完好**：`git ls-files .zcode` 仍 53 个文件，`git status` 该目录下 `D` 为 0（其 ` M` 项来自本会话早前的 Trellis 升级，与本任务无关）。

## Out of Scope

- 删除仓库根 `.zcode/`（Trellis 的 ZCode CLI 平台配置目录，与产品目标无关）。
- 移除 ZCode 相关的**站点协议/接口类型**概念本身（`zcode_api_type` 语义若仍被其他目标复用则保留其数据）。
- 任何 UI 重排、组件抽象或顺带重构（除 R2 点明的两处静默错误放大器与 R5 编译器抓不到的兜底臂）。
- 处理 ZCode CLI 自身的行为或配置。

## Decisions

- **存量策略 = 一版到底**（用户拍板）：连 `TargetKind::ZCode` 变体、`adapters/zcode.rs`、`sites.zcode_api_type` 的读写代码一起删净，不保留「只读兼容载体」，不分两个发布。前提是 R2 的升级前幂等清洗必须先于任何读取路径生效，R3 的撤退清理器必须自带。
- **分支落点 = 当前分支 `feat/site-add-presets`**（用户拍板）。已知后果：与在飞的 M1 站点预置模板、M2 魔搭余额探测共用 5 个源文件（`src-tauri/src/domain/mod.rs`、`src-tauri/src/quota_probe/mod.rs`、`src/lib/browserMock.ts`、`src/i18n/locales/zh-CN.json`、`src/i18n/locales/en-US.json`），三者发布窗口与回滚边界绑定；将来该分支并入 main 时，这 5 个文件是冲突热区。
- **物理列保留 = 证据决定，非用户决策**：`sites.zcode_api_type` 不 DROP，见 R4。
- **`~/.zcode` 撤退清理器保留 = 证据决定，非用户决策**：依据 `agents.md` 孤儿清理红线，见 R3。
