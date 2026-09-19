## 4. 目标清单的硬编码扩散

**没有 `ALL_TARGETS` 常量**（确证：`grep -rn "ALL_TARGETS\|all_targets" src-tauri/src/` 零命中；只有 `commands/mcp.rs:216` / `commands/rules.rs:32` 的同名局部函数 `merged_targets`，与全量清单无关）。→ 清单是 **9+ 份手抄数组 + 14 处 exhaustive match**，删一处漏一处不会有编译错误。这是本次移除最容易出错的地方。

### 4.1 手抄的目标数组（顺序 = 展示顺序，行为相关）

| 位置 | 用途 | 顺序依赖 |
|---|---|---|
| `commands/targets.rs:42-48` | 主界面目标状态循环 | 是（卡片顺序） |
| `commands/targets.rs:208-213` | `probe_tool` 只探 claude/codex/pi 三个 —— **本来就不含 prime/zcode**，说明 ZCode 无 CLI 探测 | — |
| `commands/apply.rs:931-939` | `list_backups` 默认目标 | **已经缺 ZCode（v0.1.5 现状）** |
| `commands/mcp.rs:355-376` | `mcp_target_paths`（设置面板展示落点） | 是 |
| `adapters/agent_rules.rs:243-250` | `target_paths` | **是，且被测试逐元素断言**（`:596-605`） |
| `adapters/mcp_scan.rs:346-352` | `scan_all` | 是（告警顺序） |
| `repo/rules.rs:12-23` | `canonical_targets` | **是，且决定落库内容**；注释（`:10-11`）明说固定顺序是为了「避免无意义的数据变更与同步指纹抖动」→ 改这个数组会**改指纹**，所有机器的 `agent_rules` 行都变，触发一次全网 push |
| `backup.rs:82-88` | `prune_all` | 否 |
| `local_proxy/mod.rs:59-65` | `target_statuses`（接管面板） | 是 |
| `tray_apply.rs:335-365` | `pick_tray_targets`，5 个位置 bool 参数 | **是**，参数顺序即目标顺序，签名改动要同步 `:403` 调用点与 `:995-1030` 测试 |
| `tray.rs:320-334` | 托盘菜单 append 顺序（`status_prime` 之后是 `status_zcode`/`status_zcode_model`） | **是**，删项会让托盘菜单少两行 |
| `tray.rs:149-153` | `format_tooltip` 6 参数 | **是** |
| `tray.rs:398-536` | `build_snapshot` 里 `find(TargetKind::ZCode)` → `zcode_line`/`zcode_model`/`tooltip_zcode` | 是 |
| `src/pages/ProxyPage.tsx:10`（前端） | `const TARGETS: TargetKind[] = [...5 个]` | 是 —— 与 `settings.localProxyTargets` 落库直连 |

### 4.2 魔数 / 计数断言（确证）

- `sync.rs:41` `const FINGERPRINT_TABLES: [&str; 9]` + `sync.rs:733` `assert_eq!(FINGERPRINT_TABLES.len(), 9)` —— **表数**，与目标数无关，本次不应改。
- `commands/settings.rs:16` 注释「剪枝要遍历 **5 个目标**（claude_code / codex / pi / prime / zcode）」—— 文档性注释，删 ZCode 后变 4，须同步。
- `adapters/agent_rules.rs:234` 注释「**五个目标**的用户级 override 设置」。
- `mcp_scan.rs:343` 与 `commands/mcp.rs:383-385` 注释写「扫描**四个**客户端」，但代码扫 5 个 —— **v0.1.5 已有的注释/代码不一致**，移除后自然对齐。
- `tray.rs:802` `assert_eq!(tip.lines().count(), 6)` —— **这就是"目标数 = 5"的隐式魔数**（1 header + 5 目标）。删 ZCode 后必须是 5。
- `types/domain.ts:21` 注释「与后端 `commands::skills::SkillTarget` 的 **5 个取值**」—— 前端，另一 agent 覆盖。

### 4.3 `SkillTarget: [SkillTarget; 5]`（`commands/skills.rs:51`）含 `agents` 不含 zcode —— 与目标数 5 恰好撞数字，**不要误改**。

### 4.4 其余零散命中（确证）

- `adapters/mod.rs:13` `pub mod zcode;`
- `quota_probe/mod.rs:3066`、`models_fetch/mod.rs:426`、`local_proxy/server.rs:494`、`adapters/{pi.rs:1007, prime.rs:1024, codex.rs:967/1050, claude_code.rs:707/1085}`、`repo/site_api_key.rs:490/538` —— **全是测试/夹具里的 `zcode_api_type: None` 结构体字面量**，删字段会编译报错，但**不涉及行为**。共 11 处。
- `deep_link.rs:190-191` —— `zcode_api_type: None` + 注释「深链导入不带 ZCode 协议选择」。**深链的 kebab 能力键里没有任何 zcode 键**（确证：`deep_link.rs` 的能力键只有 `codex-*`，见 `:419-453` 的测试；`SiteCapabilities` 是自由键 map）。
- `cli_detect.rs` —— **零 zcode 命中**（确证）。ZCode 不参与 CLI 探测。
- `app_backup.rs` / `pending_restore.rs` —— **零 zcode 命中**（确证）。应用级备份是整库 zip，与目标无关。

---

## 5. 站点预置模板冲突：**不冲突**

| 问题 | 结论 | 证据 |
|---|---|---|
| 预置模板数据里是否内嵌 zcode 键？ | **没有** | `src/lib/sitePresets.ts` / `src/lib/siteProviderKinds.ts` / `src/components/sites/AddSitePresetButton.tsx` 三文件 `grep -i zcode` **零命中**。且 `git show --stat 7579e9c` 显示 M1 只碰前端 + i18n，**没碰 `src-tauri/`** |
| ZCode 出现在 `repo/site.rs` 的 23 处是什么？ | **全是 `zcode_api_type` 这一个 `sites` 独立列的 CRUD + 测试夹具**，不是预置键也不是能力键 | `repo/site.rs:52`（`row.get(23)`）、`:89`（SITE_SELECT 末列）、`:187/:209/:327-335/:343/:360`（读写）、`:813/880/941/974/1010/1061/1160`（测试 `zcode_api_type: None`）、`:1077-1135`（专门的 round-trip 测试） |
| ZCode 是否出现在站点能力键上？ | **否**。`SiteCapabilities = HashMap<String, bool>`（`src-tauri/src/capabilities.rs:12`），自由键、无枚举，**移除 ZCode 不需要动 capabilities.rs**（确证：该文件零 zcode 命中） | — |
| 深链 `xiaobaiswitchplus://sites` 的 kebab 键导入受影响吗？ | **不受影响**。`DeepLinkSiteImportInput`（`domain/mod.rs:221-233`）**根本没有 `zcode_api_type` 字段**；`deep_link.rs:190-191` 是硬编码 `zcode_api_type: None` 并注释说明「深链不带 ZCode 协议选择」 | — |
| M2（`d945c64` 魔搭余额探测）碰过 ZCode 吗？ | **没有**。`git show --stat d945c64` 的后端改动是 `commands/quota.rs`（2 行）、`domain/mod.rs`（2 行）、`quota_probe/mod.rs`（+488 行）；`quota_probe/mod.rs` 唯一的 zcode 命中在 `:3066`，是新增测试夹具里的 `zcode_api_type: None` | — |

**结论**：本次移除与 `feat/site-add-presets` 的唯一交叉是 `src-tauri/src/domain/mod.rs`（预置分支往 `SiteRow`/`AppSettings` 加过字段，`zcode_api_type` 也在同一 struct 上）与 `src/types/domain.ts`（同一文件的 `ZCodeApiType` 与预置类型并存）。都是**文本冲突级别**，不是语义冲突。建议实现顺序上先 rebase/合并预置分支，或在 ZCode 移除里避免触碰 `SiteCapabilities` 相关代码。

---

## 6. 测试

### 6.1 应随功能一起删除（确证）

| 位置 | 测试 | 说明 |
|---|---|---|
| `src-tauri/src/adapters/zcode.rs`（`:820+` 起的整个 `#[cfg(test)]`） | ZCode 适配器全部单测（约 185 行，含 `ZCodeApiType` 断言 `:824-827`） | 删文件即删测试 |
| `adapters/mcp.rs:606` `zcode_merges_into_mcp_servers_and_keeps_other_fields` | 断言写到 `mcp.servers` 而非顶层 | 删 |
| `adapters/mcp.rs:642` `zcode_rejects_malformed_shape_and_keeps_file` | 断言形状非法时报错且保留文件 | 删（但**这条守护的是 agents.md 的兼容红线**，见 §7 —— 建议**改写**成 Pi/Prime 的同类断言，不要净失去覆盖） |
| `adapters/mcp_scan.rs:742` `zcode_reads_nested_servers_and_marks_managed`、`:761` 断言、`:765` `zcode_import_reads_from_cli_config` | 嵌套 `mcp.servers` 解析 + 纳管读盘 | 删 |
| `adapters/agent_rules.rs:615` `zcode_rules_prefer_existing_claude_md`（断言 `:625` `target_path(TargetKind::ZCode)`、`:660` `zcode/AGENTS.md` 存在） | ZCode 沿用 Pi/Prime 的 AGENTS/CLAUDE 择一规则 | 删（同规则已由 Pi/Prime 用例覆盖） |
| `repo/site.rs:1077-1135` `zcode_api_type_round_trip_and_clear` | 列的 CRUD + 空串清除 | 若保留列则**改**，若删列则删 |
| `tray_apply.rs:1032` `hydrate_zcode_falls_back_to_binding_write_all`、`:1035-1045` | ZCode 的 hydrate 回退 | 删 |
| `tray.rs:786-803` `tooltip_has_product_and_all_targets` | 传 5 个目标行 + 断言 `tip.contains("ZCode")` + `lines().count() == 6` | **改写**：去掉 zcode 实参、断言 `== 5`、断言 `!tip.contains("ZCode")` |
| `tray_apply.rs:993-1030` `pick_targets_defaults_to_both_when_unbound` | 5 个位置 bool 的逐组合断言，含 `[..., true) => vec![ZCode]` 与全 `true` 的 5 元素断言 | **改写**为 4 参数 |
| `adapters/agent_rules.rs:596-611` `target_paths` 顺序断言 | `vec![ClaudeCode, Codex, Pi, Prime, ZCode]` 逐元素 + `paths[..]` | **改写**为 4 元素 |
| `backup.rs:464-478` `parse_id_rejects_traversal` | 含 `:472-477` `parse_backup_id("zcode-1710000000000")` 断言 | 若保留清理面（§3.4）**必须保留这条断言** |

### 6.2 应新增的回归测试（当前完全缺失，强建议）

**现状（确证）**：`local_proxy/routing.rs:250-262` 的 `settings()` 夹具里 `local_proxy_targets` 只列 4 个目标（**不含 ZCode**），`:256-259` 的 `assert_same_as_direct` 也只覆盖 4 个 —— 即 `routing.rs:70` 的 ZCode 接管分支**根本没有测试**，`TargetKind::parse` 的未知字符串路径也**没有测试**。

建议补（这几条才是防止 §2.4 复发的护栏）：

1. `TargetKind::parse("zcode")` 的期望行为**被明确断言**（无论选哪条路线：`Some(ZCode)` 保留期 or `None` 终态），并断言 `parse` 对任意未知串返回 `None` 而非 panic。
2. `repo::binding::map_binding` 对未知 target 字符串的行为 —— **现在会伪装成 ClaudeCode，这是无断言的隐式契约**。至少要有一条 `INSERT INTO target_bindings VALUES ('zcode', ...)` 后断言结果的测试，把行为钉死（不管是"跳过该行"还是"识别为未知并过滤"）。
3. `repo::rules::get` / `applied_targets` / `repo::mcp::list` 在 `targets_json` 含未知值时的行为 —— 目前分别是 `[]`、`[]`、`Err`，三种不一致。建议统一为「**过滤未知项、保留已知项**」，并为三者各加一条测试。
4. 一次性清洗器的幂等性与「清洗早于 typed read」的顺序契约（§3.4 步骤 3）。

---

## 7. 兼容红线核查（对照 `agents.md`）

**先说一个重要事实（确证）**：`agents.md` 全文 `grep -i zcode` **零命中**；`.trellis/spec/` 同样零命中。即 `agents.md` 的 MCP 目标表（4 行）、全局约束目标表（4 行）、UI Shell「当前目标为 Claude Code、Codex、Pi 与 Prime」都是**四个目标** —— 文档口径早于代码。本次移除**不新增红线冲突**，反而是消除代码与文档的偏差。

### 7.1 绝对不能改的协议值 —— 以及 ZCode 代码正踩在上面的位置

| 红线（agents.md） | 代码位置 | 本次能否触碰 |
|---|---|---|
| **已应用配置命名空间 `xiaobai_`**：Pi/Prime provider id、**Codex provider id 派生**、`XIAOBAI_SITE_*`、`__xiaobai_missing__`、**MCP 服务器名** | `adapters/zcode.rs:25` `PROVIDER_PREFIX = "xiaobai_"`（**同一个命名空间**）；`is_managed()`（`:120-122`）、`prune_managed_providers()`（`:247-250`） | **只能删 ZCode 自己那份常量，绝不能改共享前缀的值**。`mcp.rs` 的 `MANAGED_PREFIX`（`mcp_scan.rs:26` 亦同值）服务于四个存活目标 |
| **MCP 服务器名写入客户端 `mcpServers` / `[mcp_servers.*]` 的键** | `mcp.rs:135-151` `merge_servers_into_json`（`managed_key(&server.name)`） | **不要动。** 特别注意 ZCode 走的是 `mcp.servers` 这个**第五个落点**，但**键名规则与其它目标同源**（都带 `xiaobai_` 前缀）。清理残留时可以复用 `MANAGED_PREFIX` 判定 |
| **备份包内数据库条目名 `xiaobai-switch.db`** | `app_backup.rs` —— **零 zcode 命中** | 与本次无关，别顺手改 |
| **本地备份前缀 `xiaobai-switch-backup-` / `any-switch-backup-` 双识别** | `app_backup.rs:17-20` `BACKUP_PREFIXES: [&str; 2]` | 与本次无关。真正与 ZCode 有关的是**目录名 `backups/zcode/`**（`backup.rs:131`）和 **id 前缀 `zcode-`**（`backup.rs:192-193`），这两个是「按 `target.as_str()` 派生」的，跟着枚举一起消失 —— 见 §3.3 的处置建议 |
| **`atomic.rs` 的 `.xiaobai-` 临时文件前缀** | `atomic.rs:16` | 共享，别动 |
| **备份痕迹识别文件名 `xiaobai-backup.json` / `xiaobai-model-catalog.json` / `.xiaobai-skill.json`** | `backup.rs:14` `META_FILE`、`atomic.rs:77` `BACKUP_ORIGINS_FILE`（`.origins.json`） | 共享，别动。ZCode 备份目录用的就是这套（`payload_files` 靠 `META_FILE`/`BACKUP_ORIGINS_FILE` 排除，`backup.rs:55-72`） |
| **`restore_official` / 孤儿清理的识别逻辑不得改动** | `commands/apply.rs:832-866`（5 臂 exhaustive）、`commands/targets.rs:92-115`（ZCode **无绑定时**靠 `live_summary().keys().any(starts_with(PROVIDER_PREFIX))` 判 Orphan）、`adapters/zcode.rs:622-624` `cleanup_orphans` | **红线本身要求「其它四个目标的识别逻辑不动」**。删 ZCode 臂不违反红线（红线针对 `xiaobai_` 前缀判定与 restore 语义），但 `mcp.rs:642` 那条「形状非法则报错并保留文件」的断言守护的是 `agents.md` 的另一条硬约束（"既有配置形状不合法时必须报错并原样保留文件"），**删测试等于删护栏**，见 §6.1 |
| **更新签名公钥 / manifest 文件名 / WebDAV 远端目录** | 与 ZCode 无交集（确证：`grep -i zcode src-tauri/src/{sync,app_backup,update*}.rs` 零命中） | 别动即可 |
| **深链 `anyswitch:` / `xiaobaiswitch:` 继续接受** | `deep_link.rs` 仅 `:190-191` 与 ZCode 有关，且是硬编码 `None` | 无冲突 |
| **`mcp_servers` 与 `agent_rules` 必须留在 `FINGERPRINT_TABLES`** | `sync.rs:41-51`、`:733-734` | **必须留**。但要注意 §2.3：表要留，**列也不能 DROP**（`sites.zcode_api_type`），否则 `SELECT *` 的列集变化会让版本偏斜的机器互相覆盖 |

### 7.2 一条 agents.md 未列但同源的约束（推测，建议按红线对待）

`repo/rules.rs:10-23` 的 `canonical_targets` 注释把「目标顺序」提升为契约（"同样的勾选集合永远产出同样的库内容，避免……同步指纹抖动"）。**从数组里删 ZCode 会改变 `agent_rules.targets_json` 的落库文本吗？** 不会 —— 因为它按固定数组 `filter(contains)`，ZCode 不在用户勾选里时输出不变。但若用户原本勾了 ZCode + Prime，落库从 `["prime","zcode"]` 变 `["prime"]` → **`agent_rules` 行内容变 → 指纹变 → 一次 push**。这是**预期且可收敛**的（不同于 §2.3 的不可收敛情况）。标注为推测：需真机双端验证一次。

---

## 风险与未决问题

### 按危害排序

| # | 风险 | 危害 | 严重度依据 | 缓解 | 状态 |
|---|---|---|---|---|---|
| R1 | `settings` blob 含 `localProxyTargets:["...","zcode"]` → `get_settings` 硬 Err → 设置页/目标状态/保存全废，本地代理与开机自启静默失效，**UI 无逃生口** | 应用变砖 | `repo/settings.rs:37` + `ProxyPage.tsx:10` 证其可达且已发布 | 阶段一**先清洗后删枚举**（§3.4 步骤 3） | **确证**（Err 传播链已逐调用点核对；具体报错文案未实跑，属推测） |
| R2 | `mcp_servers.targets_json` 含 `"zcode"` → `repo/mcp.rs:55` `?` → **一条脏行使整个 MCP 列表读不出** | MCP 功能全废，且剥夺了用户自救路径 | `repo/mcp.rs:53-55,70-77` | 同上；并把 `read_row` 改成「过滤未知 target、保留已知」 | **确证** |
| R3 | `target_bindings` 的 zcode 行 `unwrap_or(ClaudeCode)` → **伪装成 Claude Code 绑定**，污染主界面/托盘/接管面板，删站点时误删 `~/.claude/settings.json` 的鉴权 env 键并删掉真 claude_code 行 | 数据损坏 + 凭据误删，**静默无报错** | `repo/binding.rs:12` + `adapters/claude_code.rs:397-402` + `commands/sites.rs:199-234` | 阶段一 `cleanup_orphans` 后 `DELETE WHERE target='zcode'`；并给 `map_binding` 加"未知目标行跳过"语义 | **确证**（误删需同站点同时应用过 Claude+ZCode 且 key 相同，属窄条件） |
| R4 | `agent_rules.targets_json` / 两个 `applied_targets` 静默变 `[]` → **五个目标的托管块全部清不掉**（不只是 ZCode） | 全局约束功能失效 + 4 个客户端文件残留 | `repo/rules.rs:44,72` + `repo/mcp.rs:220` + `commands/rules.rs:68-70` | 阶段一清洗；把三处 `unwrap_or_default()` 改为容错过滤 | **确证** |
| R5 | `~/.zcode/v2/*.json` 的 `xiaobai_*` provider + `~/.zcode/cli/config.json` 的 `mcp.servers.xiaobai_*` 永久残留，ZCode 客户端仍会加载 | 用户机器上永远清不掉 | §3.1/§3.2 全部清理入口都在 exhaustive match 臂上 | 保留「只清理、不写入」的最小残留模块（§3.4 步骤 4） | **确证** |
| R6 | DROP `sites.zcode_api_type` 列 → `SELECT *` 列集变 → 与仍跑 v0.1.5 的对端**恒定互相覆盖**（重演 `sync.rs:53-61` 记录过的 0.1.3/0.1.4 事故） | 数据丢失（跨机） | `sync.rs:71-115` + `:53-61` 注释 | **列永久保留不 DROP**，只删代码里的读写 | **确证**（机制）/ **推测**（真机复现） |
| R7 | 9 份手抄目标数组 + 14 处 exhaustive match，**没有 `ALL_TARGETS` 常量**；数组类改动无编译保护，漏改即静默少一个目标 | 中（可测出） | §4.1 全表 | 引入 `TargetKind::ALL: [TargetKind; N]` 常量替换所有数组（**独立小 PR，先做**），再删变体时一次性红所有点 | 确证 |
| R8 | 删 `backup.rs:259-264` 的 `mapped_dest` ZCode 臂（有 `_ => None` 兜底）却保留 `parse_backup_id` 的 `zcode-` 前缀 → 恢复时报 `"no restorable files"` 而非清晰错误 | 低（误导排障） | `backup.rs:195-197,240-266,346-349` | 两函数同进退 | 确证 |
| R9 | 删 `prune_all` 的 ZCode 臂 → `~/.xiaobai-switch/backups/zcode/` 永远堆积（v0.1.5 已把它从 UI 列表里漏掉，用户看不见也删不掉） | 低（磁盘） | `backup.rs:86` vs `commands/apply.rs:935-939`，v0.1.5 同状态 | 保留 prune 或删除该目录并在 release note 说明 | 确证 |
| R10 | `mcp_scan.rs:168-179` 两个 match 有 `_` 兜底 —— 删 `ScanTarget::ZCode` 后**编译通过、语义静默改变、unreachable code** | 低（排障困难） | §3.2 | 改 `_` 为穷尽 match，让编译器参与 | 确证 |

### 未决问题（需要产品/主 agent 决定，我没有依据判断）

1. **`~/.zcode` 目录本体要不要删？** 那是 ZCode 应用自己的配置目录，本应用只往它的 `v2/`、`cli/` 里塞过条目。**建议：只删自己的条目，绝不动目录或其它 provider。** 与 `agents.md` 的「`restore_official` / 孤儿清理的识别逻辑不得改动」以及「不要动用户自己的条目」同源。但**没有任何文档规定跨应用卸载边界** —— 需要产品明确。
2. **阶段一与阶段二是否分两个 release？** 若合并成一个 PR，R1/R2/R3 会在**同一次升级**同时命中真实用户，且清洗器必须在同一个版本里跑赢所有 typed read。我倾向分开，但这取决于发布节奏与 v0.1.5 的实际装机量 —— 我没有装机量数据。
3. **`apply_records` 里 `target='zcode'` 的历史记录如何展示？** 它是 `String`（`domain/mod.rs:974`），不会报错，但前端 `TargetStatusCard`/历史列表会拿到一个不认识的标签。需要前端决定（fallback 标签 or 过滤）。
4. **验证 `unwrap_or_default` → `[]` 的实际后果需要跑一次**（我没跑 cargo）：构造 `agent_rules.targets_json='["claude_code","zcode"]'`，读 `repo::rules::get`，断言 `targets`。这是 R4 的直接复现，成本很低，建议实现前先做。
5. **`canonical_targets` 数组改动是否触发全网 push（§7.2）**：需要双机真机验证一次；若不希望触发，可让 `canonical_targets` 保留 `ZCode` 在数组里（读时永不命中，零行为差异），仅从 UI 清单移除。

### 建议实现顺序（供主 agent 编排参考，非本 agent 产出）

0. 独立前置 PR：把 §4.1 的 9 份手抄数组收敛成 `TargetKind::ALL` 常量（**不含任何行为变化**，可先合、可回归）。
1. 阶段一 PR：`TargetKind::ZCode` 保留；从 `ALL` / 9 份清单里摘掉；新增启动清洗器（裸 `Value` 操作，先于任何 typed read）；三个 `unwrap_or_default`/`unwrap_or` 改为容错过滤；补 §6.2 的 4 条回归测试。
2. 真机验证：造一个 v0.1.5 脏库快照，升级，确认设置页/MCP 页/全局约束页可打开、本地代理仍自启、`~/.zcode` 条目已清、无 `~/.claude` 误删。
3. 阶段二 PR（隔 N 个版本）：删 `TargetKind::ZCode`、`ScanTarget::ZCode`、`zcode.rs`、14 处 match 臂、`SiteDto/CreateSiteInput/UpdateSiteInput/AppSettings` 的 zcode 字段、tray 的 3 个结构字段与 6 参数 tooltip、11 处测试夹具的 `zcode_api_type: None`。**`sites.zcode_api_type` 列保留不 DROP**；`paths::zcode_*` 与 `parse_backup_id("zcode-")` 视 §3.4 决定。
