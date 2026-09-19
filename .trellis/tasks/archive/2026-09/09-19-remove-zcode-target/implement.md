# 执行计划：移除 ZCode 应用目标

依赖 `prd.md`（R1–R9 / AC1–AC11）与 `design.md`（U1–U4、§3 契约、§6 回滚）。

按文件组织的精确删除清单**不在此重复**。因 `context_injection.max_file_bytes = 32768` 会截断超长文件，四份研究均已切在限内，必须成对读：
- `research/backend-data-compat.md`（§1–§3：目标枚举穷举点、四个读取失败模式、`~/.zcode` 落点）
- `research/backend-target-surface-and-redlines.md`（§4–§7：硬编码目标数组与顺序、魔数、测试、**兼容红线核查**、R1–R10 风险表）
- `research/frontend-surface.md`（§1–§8：三份并列联合、组件与 stores、i18n、mock 锁步表、隐式顺序依赖）
- `research/frontend-removal-worklist.md`（**按文件可勾选移除清单**，兜住编译器照不到的四处静默点）

分支：当前 `feat/site-add-presets`（用户决定）。**开工前先处理一件事**：本分支相对 main 已改 112 个文件且工作区有 189 个 stat 脏项（内容零变更，见本会话调查）。先确认 M1/M2 已提交，再开始，避免撤退改动与未提交工作混淆。

---

## 阶段 0：立守门测试（先红后绿）✅ 已完成

> 实测修正：夹具里的 JSON 数组站点真实存量形状是 **`"z_code"`**（serde 通道），不是这里原写的 `"zcode"`（`as_str()` 通道）。两种拼写都造了。依据与影响见 design.md §3.1。

- [x] 0.1 造 v0.1.5 形状数据库夹具：设置 blob 的 `localProxyTargets` 含 `"z_code"`；`mcp_servers.targets_json`、`agent_rules.targets_json`、`sync_meta.agent_rules_applied_targets` + `mcp_applied_targets` 各含 `"z_code"`；`target_bindings` 与 `apply_records` 各 ≥1 行退役目标；`sites.zcode_api_type` 非空。
- [x] 0.2 写断言但**先不实现**：`get_settings` 可读可存、`repo/mcp.rs` 列表可读、`repo/rules.rs` targets 非空、退役目标行消失、`zcode_api_type` 为 NULL、列仍存在、`SCHEMA_VERSION` 与 `FINGERPRINT_TABLES` 未变。→ AC1 / AC3 / AC4 / AC6。
- [x] 0.3 写假 `~/.zcode` 夹具（tempdir + 复用既有 `Settings.zcode_home_override` 注入，未新增生产字段、未碰真实用户目录）：三处 config 含 `xiaobai_*`、`AGENTS.md` 含托管块且块外有用户内容、另一份带 UTF-8 BOM、再一份「只有 BEGIN 没有 END」的畸形文件。→ AC5。

```bash
cd src-tauri && cargo test zcode_retirement -- --nocapture   # Gate 0 实测：18 failed / 7 passed
```

**Gate 0** ✓ 红灯成立。那 7 条通过的正是「不该写」的负向用例（无我们的东西就不动盘、自由文本不算脏、缺目录全程静默），符合预期。

## 阶段 1：U1 数据撤退层 ✅ 已完成（Gate 1 通过）

- [x] 1.1 新建 `src-tauri/src/zcode_retirement.rs`（约 845 行生产码 + 845 行测试），文件头注释写明存在理由与「何时可整文件删除」的三条前置条件。自包含：只认字面量，不 import `adapters::zcode`、不用 `TargetKind::ZCode`。
- [x] 1.2 实现 `ensure_in_db(conn)`：先 `build_plan` 全量只读扫描，`Plan::is_empty()` 则一个写都不发；否则先整库快照再 `apply_plan`。数组类走 `Value` 递归往返（不认键名、只认数组元素值），行类 `DELETE`，`zcode_api_type` 置 NULL，不递增 `SCHEMA_VERSION`。
- [x] 1.3 挂载到 `ensure_incremental_schema`（`db/migrate.rs`），三个分支的测试各自独立证明挂到了（`mounted_on_the_{up_to_date,legacy_migration,fresh_schema}_branch`）。**清洗失败在挂载点降级为 warn**：冒泡出去会变成「数据库打不开」，代价是清洗下次启动重试。
- [x] 1.4 写变更前整库备份。抽出 `db::migrate::backup_snapshot(conn, subdir, replace_existing, require_master_key)` 供 `backup_pre_migration` 与撤退共用；撤退侧 `replace_existing=false`（要留的是**清洗前**那份）、`require_master_key=false`（缺 key 只 warn）。
- [x] 1.5 `clean_external_once(override)`：四类落点 + 四条清理规则；`FileLock` 互斥、`backup_file` 写前备份、`atomic_write` 原子替换；无变化不备份不写盘。**未加 `sync_meta` 完成标记**（§5 表）。空白 override 视为错误，绝不静默回退到真实 `~/.zcode`。
- [x] 1.6 在 `lib.rs` `setup` 接 1.5，位置在 `AppState::init()` 之后、托盘与代理前；未放进 `apply_schema`。
- [x] 1.7 修 §3.3 静默放大器：`repo/binding.rs` 的 `map_binding` 改回 `Option`（未知目标跳过并 warn，不再冒充 Claude Code）；新增 `domain::parse_persisted_targets(json, context)` 做逐元素解析，替换 `repo/rules.rs:44/72`、`repo/mcp.rs:220` 三处 `unwrap_or_default()`。
  > 范围外溢一处（已确认必要）：`repo/mcp.rs:220` 的 `applied_targets` 与 rules.rs 是同一个「脏数据静默清空全部目标清理集合」缺陷，同批修掉；design §3.3 原文只列了两处。

```bash
cd src-tauri && cargo test zcode_retirement   # 25 passed / 0 failed
cd src-tauri && cargo test                    # 592 passed / 0 failed / 2 ignored
```

**Gate 1** ✓。变异验证（逐条破坏实现再恢复，确认非空断言）：

| 破坏点 | 变红的用例 |
|---|---|
| `map_binding` 退回 `unwrap_or(ClaudeCode)` | `unknown_target_binding_is_skipped_instead_of_masquerading_as_claude_code` |
| `strip_managed_keys` 去掉 `xiaobai_` 前缀判断（全删） | `managed_provider_entries_are_pruned_and_user_entries_survive` + `nothing_is_written_when_there_is_nothing_of_ours` |
| `strip_managed_keys` 改成 no-op（不删） | `managed_provider_entries_are_pruned_and_user_entries_survive` |
| `backup(conn)` 挪到 `apply_plan` 之后 | `retirement_is_idempotent_and_backs_up_exactly_once`（靠 `BackupProbe.still_dirty` 探针） |
| `strip_block` 缺 END 时从 `Err` 改成截断 | `malformed_targets_are_skipped_and_left_untouched` |

**本机工具链缺口（待确认，勿当已验证）**：`cargo clippy` 未安装（`rustup component add clippy` 未执行，装系统依赖须先问用户）。因此阶段 3 的 `cargo clippy -- -D warnings` 与 Gate 2 的「零新增警告」只能用 `cargo check --lib --tests` 的 rustc 警告近似核对 —— 已确认本次改动的 7 个文件零 rustc 警告，但这不等于 clippy 干净。
> 该缺口已在收尾时经用户批准补装并核完，实测结论见阶段 5 末「工具链核查」；上面这句保留为当时的事实，不要引用为当前状态。


## 阶段 2：U2 目标模型收缩 ✅ 已完成（Gate 2 通过）

- [x] 2.1 删 `domain/mod.rs` 的 `ZCode` 变体、`as_str` 与 `parse` 分支。
- [x] 2.2 删前端三份并列联合成员：`src/types/domain.ts` 的 `TargetKind`、`src/stores/uiStore.ts` 的 `ApplyTargetTab`、`src/types/mcp.ts` 的 `ScanTarget`（含注释「五个」→「四个」）。三份同批改，`McpPage.tsx` 的 `entry.target as TargetKind` 断言因此仍然成立。
- [x] 2.3 编译器扫后端穷举 `match` 逐个补齐；`adapters/mcp_scan.rs` 两个带 `_` 兜底的 `match` 按清单人工复核（兜底分支删不掉，只能靠人确认语义）。
- [x] 2.4 `pnpm typecheck` 扫前端穷举结构（3 个 `Record<TargetKind, string>` + `MENU_ICONS` 精确报错，逐个补齐）。

```bash
cd src-tauri && cargo check --lib --tests   # 0 errors
pnpm typecheck                              # 零错误
```

**Gate 2** ✓。**口径纠正**：`cargo check --lib --tests` 并非「零警告」—— lib 目标稳定输出 **54 条 rustc 警告**。`trellis-check` 逐条与 HEAD 比对（`max_copies`、`targets_to_prune`、`BodyExt`、`entry_fingerprint`、`cfg(unix)` 下的 `dir_created` 等），确认**本任务引入为零**，其余为既有或平台门控警告。「零新增警告」成立，「零警告」不成立。

## 阶段 3：U3 后端消费面 ✅ 已完成（Gate 3 通过）

- [x] 3.1 删 `src-tauri/src/adapters/zcode.rs`（1005 行）与 `adapters/mod.rs` 的 `mod zcode;`。
- [x] 3.2 按 §4 清单逐个收口：`commands/{targets,apply,sites,settings,mcp,rules}.rs`、`tray.rs`、`tray_apply.rs`、`route_switch.rs`、`key_switch.rs`、`local_proxy/{mod,routing,server}.rs`、`quota_probe/mod.rs`、`models_fetch/mod.rs`、`deep_link.rs`、`adapters/{mcp,mcp_scan,agent_rules}.rs`、`backup.rs`、`paths.rs`、`repo/{site,site_api_key,rules,binding}.rs`。
- [x] 3.3 `paths.rs` 删掉 5 个 ZCode 专用函数（`default_zcode_home` / `resolve_zcode_home` / `zcode_provider_path` / `zcode_provider_config_path` / `zcode_mcp_path`）；撤退所需常量只留在 `zcode_retirement.rs` 内部。
- [x] 3.4 `commands/skills.rs` 的 `SkillTarget` 未含 ZCode —— 确认未误改。
- [x] 3.5 Rust 测试收口（见下方偏离清单）。

```bash
cd src-tauri && cargo test    # 579 passed / 0 failed / 2 ignored
```

**Gate 3** ✓。579 比阶段 1 的 592 少 13 条 —— 全部是随被删功能一起消失的 ZCode 专用断言，不是削弱。`zcode_retirement` 模块本身 25 → **27**。

**与计划的偏离（都已核实）**：

- `tray.rs` 的 tooltip 测试改成守门形态：`assert!(!tip.contains("ZCode"))` + `assert_eq!(tip.lines().count(), 5)`，并在 `handle_menu_event` 删掉不可达的 `status_zcode*` 臂。
- `zcode_retirement` 的一条 MCP 前置断言**反转**而非放宽：U2 之后「库里还留着 zcode 条目」在设计上不可能成立，原前置条件失效；新形式钉住的是「先清洗、后写盘」这条顺序保证。
- 删 1 条 `the_blank_override_never_falls_back_to_the_real_home`，由更宽的 `a_missing_or_blank_legacy_override_reads_as_none` 覆盖同一不变式。
- **新增自定义目录恢复通道**（计划外、修补缺口）：`AppSettings.zcodeHomeOverride` 作为 Rust 字段已删，但设置 blob 是 `serde` 宽松反序列化，历史键 `zcodeHomeOverride` 会一直留在 JSON 文本里直到第一次 `save_settings`。`zcode_retirement::leftover_home_override` 直接读原始 blob 取回它，`clean_external_once` 因此也能清掉「把 ZCode 目录指到别处」的用户机器上残留的明文 API Key —— 没有为此重新引入生产结构体字段。配套 3 条新用例（`legacy_override_key_survives_the_scrub`、`a_missing_or_blank_legacy_override_reads_as_none`、`the_same_root_is_only_cleaned_once`）。
- `repo/site.rs`：`SITE_SELECT` 收口为 23 列、INSERT 16 列，`zcode_api_type` 不再读写，但**列与 DDL 保留**（逻辑指纹按 `SELECT *` 逐列哈希）。

## 阶段 4：U4 前端与契约 ✅ 已完成（Gate 4 通过）

- [x] 4.1 删 `ZCodeApplyPanel.tsx` + 其 `.test.tsx`（整文件）；删 `ApplyPage.tsx` 第 5 段 JSX 与 import。
- [x] 4.2 删 `hydrateApplyForm.ts` 的整段 zcode 导出（`ZCODE_API_TYPES` / `inferZCodeApiType` / `parseZCodeApiType` / `ZCodeFormDefaults` / `hydrateZCodeForm`）与 `domain.ts` 的 3 个可选字段 + `ZCodeApiType`。顺带删掉因此不再被使用的 `SiteProtocol` import。
- [x] 4.3 编译器照不到的三点：**`targetKindLabelKey` 默认臂改为 `never` 穷尽性守卫**（见未决问题 1）；`showApplyOutcome.applyResultBodyKey` 同样处理；`ProxyPage.tsx` 三处补 `?? target` 兜底（与另两页对齐，R3）。
- [x] 4.4 收口 `ApplySidebar.tsx`（含 `SquareTerminal` import）/`ApplyFooter.tsx`（三元链三处臂）/`SettingsPage.tsx`（`PathsSection` 的 state、effect、依赖数组、保存实参、输入块）/`McpPage.tsx`/`ProxyPage.tsx`/`RulesPage.tsx`/`stores/{apply,ui,settings}.ts`。
- [x] 4.5 i18n：中英各删 **24 个键**（清单的 22 个 + 未决问题 2 的两条死键），脚本化删除并校验「前后键集只差这 24 个」（脚本：`research/i18n_key_scrub.py`，改前 dry-run 会打印漏配与逗号风险），中英对称，`zcode` 零命中。
- [x] 4.6 `browserMock.ts` 12 个位点与真实 command 同步删除。
- [x] 4.7 测试收口：`proxyStore.test.ts`（目标数组 5→4）、`ProxyPage.test.tsx`（用例名 five→four、开关数 6→5、删 2 条断言）、`McpPage.test.tsx`（纳管按钮 3→2、**「全部纳管后 servers 3→2」不在原清单里，跑测试才暴露**、删 1 条断言 + 改注释）、`ApplySidebar.test.tsx`（删整条用例）、`showApplyOutcome.test.ts`（删 1 条）。
- [x] 4.8 `packagingWorkflow.ts` 的 "targets" 是 Rust target triple 假阳性 —— 确认未误改。
- [x] 4.9 `README.md` / `README_EN.md` 各 4 处收口为四目标口径；**第 193 行 `.zcode/tasks/` 保留**（那是 Trellis 的 ZCode 平台目录，与产品目标无关）。

```bash
pnpm typecheck    # 零错误
pnpm test:run     # 432 passed / 0 failed，55 / 57 文件通过
```

**Gate 4** ✓。2 个收集失败文件是**既有问题**、与本任务无关：`generateUpdaterManifest.test.ts` 与 `validateUpdaterSigningSecret.test.ts` 对 `scripts/*.mjs` 做顶层 `await import`，报 `SyntaxError: Invalid or unexpected token`；这 4 个文件本任务零改动。

**三个未决问题的落点**：

1. `targetKindLabelKey` 默认臂 → `const unreachable: never = kind; return unreachable;`。**不**指向任何存活目标：真在运行时收到本版本不认识的目标名（旧对端推来的残留）时宁可原样回显裸 token，也不能冒充别的目标，否则用户会把一个目标的数据看成另一个目标的。
2. `apply.dualWarning` → 与 `apply.dualWarningTitle` 一并删除。两条都是死键（全仓库只有 `sites.goApply` 被 `GoApplyButton.tsx` 用到），改文案给死键续命没意义。同族死键 `sites.goApply{Claude,Codex,Pi,Prime}` 不动 —— 与本任务无关。
3. `README*` 同批删（PRD 只写「前端面板与文案」，但 README 是用户可见的支持清单，留着即错误宣传）。

**顺带发现（未处理，超出本任务范围）**：`src/i18n/locales/{zh-CN,en-US}.json` 的顶层 `rules` 与 `proxy` 各出现两次（约 794/871、822/935 行），两份内容逐键相同，`JSON.parse` 后写覆盖前者 → **当前无行为影响**，但删键必须在 4 个块里各删一次才会真消失（本次已如此）。建议单独立任务合并。

## 阶段 5：收尾核查 ✅ 全部完成（真机项见 5.5，托盘一项的证据级别已标注）

- [x] 5.1 仓库级 grep（排除 `.git` / `node_modules` / 构建产物 / 各平台配置目录）后只剩：`zcode_retirement.rs`（本体）、`db/migrate.rs`（保留的 DDL + 挂载注释）、`lib.rs`（挂载点）、`backup.rs`（`parse_backup_id` 回归断言）、`tray.rs:747`（守门断言）、`repo/binding.rs`（注释）、`.gitignore` 与 `README*:193`（Trellis 的 `.zcode/`）。**写入型路径为零**。
- [x] 5.2 `grep zcode src/i18n/locales/` —— 中英双侧零命中。
- [x] 5.3 仓库根 `.zcode/` 未被删除；全仓库 `git status` 的 `D` 只有 3 条，都是本任务目标的源码文件。
- [x] 5.4 `sites.zcode_api_type` 的建表列（`migrate.rs:61`）与增量 `ALTER`（`:209-210`）都在；`sync.rs` 本任务**零改动**（`FINGERPRINT_TABLES` 仍 9 项、`FINGERPRINT_ALGORITHM_VERSION` 仍 1）。
- [x] 5.5 **真机冒烟**：打包版已在本机构建 + 安装 + 启动，界面（除原生托盘）已逐页核对。原状态是「没有跑起打包后的应用」，收尾时经用户要求在真机构建并安装了当前分支（HEAD `3c7c971`，其代码内容与 `cc9b437` 逐字节相同 —— 其后两个提交只动 `.trellis/`）。等价自动化覆盖：v0.1.5 形状夹具上的 `ensure_in_db`（27 例，含设置 blob / 三个 targets 数组 / 两类绑定行 / 列置空）、tempdir 假 `~/.zcode` 四类落点清理（含 BOM、块外内容、畸形标记）、托盘文案与线数断言。

  **真机结果（2026-09-19，用户自己的机器与真实数据库）**：
  - 组装：`tauri build --bundles nsis --config '{"bundle":{"createUpdaterArtifacts":false}}'`。关 `createUpdaterArtifacts` 是因为 `tauri.conf.json` 里它为 `true`，而 minisign 私钥只存在于 CI secret，本机没有 —— 不是配置问题，别照字面在本机跑默认配置。产物 `XiaoBaiSwitch Plus_0.1.5_x64-setup.exe`（9,143,754 B），静默安装退出码 0，落到 `D:\Program Files\XiaoBaiSwitch Plus\`。
  - 装后校验：安装目录的 `XiaoBaiSwitchPlus.exe` 与构建输出逐字节比对，差异**恰好 3 字节**（偏移 `0x144bd12`，`UNK` → `NSS`），即 NSIS 打包器往 PE 资源段打的补丁，不是代码差异。
  - 启动：`lib.rs::setup` 的真实启动路径在**用户真实库**上跑通了 —— 应用起来了（托盘常驻，`visible: false`，二次启动走单实例回调 `restore_main_window`）。这是本任务唯一一条真机端到端证据，闭掉了 5.5 原本的「`setup` 未跑」缺口。
  - 撤退清洗在干净存量上**零写入**（与 design §6 的预期一致）：以安装前 278 文件基线对比安装并运行后的 `~/.xiaobai-switch` + `~/.zcode`，新增 **0**、删除 **0**，主库 `xiaobai-switch.db` sha256 **不变**（只有 `-wal` / `-shm` 随正常读写变），`backups/` 下**没有** `zcode-retirement/` 快照目录 —— 清洗器判定无行可改，就没有触发那次「写前整库快照」。
  - 已知无害残留（真机复现了单测里的那条口径）：`zcodeHomeOverride` 虽已从 `AppSettings` 结构体删除，仍以 `"zcodeHomeOverride": null` 留在设置 blob 文本里（无 `deny_unknown_fields`，反序列化忽略未知键），blob 其余 25 键解析正常。它在下一次设置保存时被抹掉，不需处理。
  - `~/.zcode/v2/provider_config.json` 安装后确实变了，但改动方是 **ZCode 自己**（mtime 15:50:37，内容零 `xiaobai` 痕迹），与本任务无关，别记成撤退清洗写的。
  - **AC7 已核对**（`pnpm dev` + 真浏览器跑同一套 React 组件，走 `browserMock` 层）：六个页面（站点 / 应用中心 / 技能 / MCP / 全局约束 / 本地代理）+ 设置五节逐页扫 `document.documentElement.outerHTML` 的 `/zcode/i` → **零命中**；每页目标复选框集合实测恰为 `Claude Code / Codex / Pi / Prime`；应用中心侧栏、MCP「手动添加」弹窗的「应用目标」、全局约束「生效目标」、本地代理目标组、设置「路径」节（原 ZCode 主目录覆盖项已消失）逐个抓到。驱动方式是 `evaluate_script` 里 JS `click()` —— 本机 computer-use 的坐标点击落不进 WebView2 内容区（DPI 缩放），指针级操作不可用，所以出的是 **DOM 证据**而非截图证据。
  - **托盘子菜单仍无肉眼证据，只有结构性 + 断言证据**：`build_menu`（`tray.rs:299-314`）是**逐目标硬写**的状态行，ZCode 那行连同 `TraySnapshot` 的对应字段一并删了 —— 没有字段可渲染，属编译期保证；`tray.rs:747` 再断言行数与 tooltip。托盘是原生菜单，浏览器路径覆盖不到，本机也没抓到可信截图。**这一项别记成「看过界面」。**
  - 一处交接注意：本机装的这版 `tauri.conf.json` 里 `version` 仍为 `0.1.5`（本任务不升版本号），与 GitHub 上已发布的 0.1.5 同号。界面上看不出区别，只有文件哈希能分 —— 后续真正发版时版本号必须往前走，否则用户无从判断自己装的是哪个。
- [x] 5.6 全量：`cargo test` 579/0/2、`pnpm typecheck` 零错误、`pnpm test:run` 432 passed。

**发布说明：核实过生成机制，结论是不用手写文件**（收尾时按「还剩发布说明」查的）。Release 正文由 CI 的 `orhun/git-cliff-action@v4` + `cliff.toml` 生成（`args: --latest --strip header --offline`，即「最近一个 `v*` tag 之后」）：

- 模板每条只取 **commit 主题行**（`commit.message | split(pat="\n") | first`）+ scope，按 `commit_parsers` 分组。所以用户会看到的那一行就是 `feat(targets)!: 移除 ZCode 应用目标，存量与落盘痕迹一次性收口` → 归入「🚀 新功能」；`!` **不会**渲染成 BREAKING 标记（模板不读 `breaking_description`），破坏性只能靠主题行自己说清 —— 本条的「一次性收口」已表达了「无过渡版本、升级即清洗」。
- 仓库里没有 CHANGELOG 文件，也不该新增：正文全新生成，手写不进流水线。
- 两个流水线事实供发版时决策，本任务不动：① `filter_unconventional = false` 且 `^chore` / `^docs` / `^style` 都有分组，本次 6 条提交里 4 条内部提交（trellis 文档、归档、日志、clippy 样式）会原样进用户可见的发布说明；② `publish-release` 会用同一份生成内容**覆写**草稿正文，所以在 GitHub 上手工编辑草稿是白改 —— 要加人工说明只能往 commit 主题或 `cliff.toml` 里想。
- 本机未装 `git-cliff`（不静默装系统依赖），以上是按模板人肉推的，**不是渲染实测**。

**顺带发现（未处理，与本任务无关）**：真机 DOM 走查时看到两处用户可见文案只列了三个目标 —— `app.tagline`（zh「一键接入 Claude Code / Codex / Pi」/ en "…for Claude Code, Codex & Pi"）与 `onboarding.welcomeDesc`（zh「接入 Claude Code、Codex 或 Pi。」/ en "…for Claude Code, Codex, or Pi."），中英**双侧都漏了 Prime**。`git log -L` 证明这两行停在 `fe5c8e8`（接 Pi 那版），是 **Prime 落地时漏改**，不是本任务删 ZCode 删坏的。建议单独小任务补。

**工具链核查（收尾时经用户批准 `rustup component add clippy rustfmt`：clippy 0.1.98 / rustfmt 1.9.0-stable）**：两项都按「只算本任务引入的增量」口径做，不做全仓清洗。

- **clippy —— 门禁已真正闭合**。`cargo clippy --offline --lib --tests --message-format=short` → 0 error、131 条唯一警告（HEAD 本身就有 ~122 条：仓库从未跑过 clippy）。把 `git diff -U0 f54aca1^..f54aca1` 的新增行区间与警告的 `file:line` 求交集，**首查命中 2 条，都在新增文件里**：`zcode_retirement.rs:230` `manual_contains`（改 `RETIRED_JSON_NAMES.contains(&value)`）、`:445` `doc_lazy_continuation`（列表项后的续行前补一条空 `///`）。修完复跑交集 **0 命中**；既有 ~122 条按「不顺手重构」不动。
  → 推论：**`cargo clippy -- -D warnings` 不能当本仓门禁**，它会在与本任务无关的既有码上红。原计划阶段 3 那一行按交集口径执行，不照字面跑。
- **rustfmt —— 不是本仓约定，故不执行**。仓库无 `rustfmt.toml`（根与 `src-tauri` 均无），`.github/workflows/` 对 fmt / clippy / rustfmt **零命中**。实测 15 个被改 `.rs` 文件中 **10 个整体不合 rustfmt 默认**，仅 `lib.rs` 就有 795 行 rustfmt 想动，而它本次只新增 13 行 —— 跑 `cargo fmt` 会产出巨量无关清洗，违反范围红线。落在新增行上的 rustfmt 意见逐条抽检（`chain_width` 默认 60 触发链式换行、`max_width` 按 CJK 列宽计）均为默认口径差异，非风格退化。
  > **方法学教训（已写进 `.trellis/spec/backend/index.md`「提交前机械检查」）**：`rustfmt --check` 的输出头是它自己的 `Diff in <path>:<line>:`，**不是** unified diff 的 `@@ -a,b +c,d @@`。用 `@@` 去解析必然得到「0 命中」的**假阴性** —— 本仓 rustfmt 大面积不合默认，任何此类交集脚本都要先打印中间量、确认解析到了行号再信结论。第一版脚本就中了这一枪。
- 两处 clippy 修复后复跑全量：`cargo test` **579 passed / 0 failed / 2 ignored**，与 5.6 一致，无回归。

**`trellis-check` 复核结论（独立子代理，6 项定向核查）**：跨层契约、静默死代码、兼容红线、守门测试是否放宽、i18n 孤儿与中英对称、前端可达入口 —— **6 项均无发现，零改动**。附带两条信息：① 上面那条 Gate 2「零警告」口径不成立，已纠正；② i18n 按**唯一键路径**统计为中英各 887 键且集合完全对称、代码引用零缺键（R6 原文的 894 是含重复块的逐行计数，两种口径都对，别混用）。


## 风险点与回滚

| 项 | 说明 |
|---|---|
| **最高危** | 阶段 1→2 的中间态。U1 未绿就删枚举 = 构建出的程序打不开老用户库。Gate 1 是硬闸。 |
| **静默语义变化** | `ScanTarget`（`_` 兜底）、`hydrateApplyForm` 的 `export`、`TargetStatusCard:302` 默认臂、`browserMock` 无 `deny_unknown_fields`。四处编译器全部照不到，只能靠清单 + 守门测试。 |
| **有损不可逆** | `target_bindings` / `apply_records` 的 `DELETE`。缓解见 design §6（清洗前整库备份）。 |
| **冲突热区** | 与 M1/M2 共用 `domain/mod.rs`、`quota_probe/mod.rs`、`browserMock.ts`、中英 i18n 共 5 文件。撤退改动尽量集中提交，便于将来单独 cherry-pick。 |
| **回滚** | 代码 `git revert`；数据靠 `~/.xiaobai-switch/backups/` 的清洗前快照。 |

## `task.py start` 之前

- [x] 阶段 0 的夹具与红断言已就位（Gate 0 红灯成立后才 start）。
- [x] `implement.jsonl` / `check.jsonl` 已含真实 spec/research 条目。
- [x] 本计划经用户批准（「批准」），并确认三项用户决定：彻底删干净（一版到底）、就在 `feat/site-add-presets` 分支做、目标名是 zcode。

