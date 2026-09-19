# Research: 前端侧 ZCode 移除面与 invoke 契约同步

- **Query**: 从前端（`src/`）完整移除 ZCode 应用目标的影响面、穷举式契约、mock 同步、测试面
- **Scope**: internal（前端为主；为对齐 invoke 契约只读了 Rust command 签名/serde 形状，未做后端业务分析）
- **Date**: 2026-09-19

引用来源：`git show --stat 29284d7`（前端接入提交，26 文件）与 `21782f0`（后端接入提交）。当前 HEAD 命中 **25 个 `src/` 文件**（`GoApplyButton.tsx` 已在 `ef5c761` 被改回通用按钮，故不再命中）。

---

## 结论摘要

1. **穷举结构充足，绝大多数遗漏会被 `tsc` 拦下**。`TargetKind`（`src/types/domain.ts:15`）被 3 个 `Record<TargetKind, string>` 全映射（McpPage/ProxyPage/RulesPage 的 `TARGET_LABEL_KEYS`）和多个 `TargetKind[]` 字面量数组穷举；`ApplyTargetTab`（`uiStore.ts:7`）被 `MENU_ICONS: Record<ApplyTargetTab, React.ReactNode>` 穷举。**从联合里删 `'zcode'` 后，这些映射里残留的 `zcode:` 键会因「对象字面量超额属性检查」直接报错**，`"zcode"` 字面量数组元素与 `target === "zcode"` 比较同样报错（TS2367/TS2353）——不存在「删了类型、用到它的地方静默放过」。
2. **三处 `noUnusedLocals` 会强制你删干净 import**（`tsconfig.json:15`）：`ApplySidebar.tsx:5` 的 `SquareTerminal`、`ApplyPage.tsx:9` 的 `ZCodeApplyPanel`、`hydrateApplyForm.ts:9` 的 `SiteProtocol`（该类型只被 `inferZCodeApiType` 用）。
3. **但 `noUnusedLocals` 不管 export**：`hydrateApplyForm.ts` 里 `ZCODE_API_TYPES` / `inferZCodeApiType` / `parseZCodeApiType` / `ZCodeFormDefaults` / `hydrateZCodeForm`（110–139 行）**全是导出符号，删掉面板后变成静默死代码，必须手工删**。这是本次最大的「不会编译报错的遗漏」。
4. **两处「兜底臂恰好就是 ZCode 标签」的 fall-through 是隐式错位点**：`TargetStatusCard.tsx:298-302`（`targetKindLabelKey` 的 `return "apply.targetZCode"` 是默认分支）与 `showApplyOutcome.tsx:29-33`（默认分支是 `resultPiOk`，zcode 是显式臂）。前者删 i18n 键后会把字面量 `apply.targetZCode` 渲染到 UI 上。
5. **`src/` 下没有任何 zustand `persist`、也没有 target 相关的 localStorage**（仅 `routeProbe.ts` / `siteIcon.ts` 两个缓存键）。**老用户本地不会残留 zcode 状态**；所有 zcode 存量都在 SQLite（后端 agent 范围）。所以「hydrate 报错/脏键」这个担心在前端不成立。
6. **i18n 两侧完全对称：zh-CN 与 en-US 各 22 个 key 路径含 `zcode`，无单侧键**。另有 1 个**已被共享复用、key 路径不含 zcode 但文案提到 ZCode** 的键 `apply.dualWarning`（值是「…Claude Code、Codex、Pi、Prime 与 ZCode 同时接受…」）——按 key 名过滤删键会漏掉它。`sites.goApplyZCode` 已是死键（连同 `sites.goApplyClaude/Codex/Pi/Prime` 全家族都是死键）。
7. **browserMock 与 Rust 是逐命令契约**：10 个 mock 位点带 zcode，对应真实 command `get_settings`/`save_settings`、`list_target_status`、`apply_site`、`revert_target`/`restore_official_target`、`detect_cli_tools`、`local_proxy_status`、`scan_existing_mcp`/`import_scanned_mcp`、`mcp_target_paths`、`get_agent_rules`/`save_agent_rules`/`agent_rules_target_paths`、`create_site`/`update_site`。**必须与 Rust 同批删**，否则 mock 会在浏览器里造出一个真后端已不存在的目标（详见小节 6 的锁步表）。
8. **测试面 6 个文件**：`ZCodeApplyPanel.test.tsx` 整体删；`ApplySidebar.test.tsx`（第 3 个用例）、`showApplyOutcome.test.ts`（1 行）、`proxyStore.test.ts` + `ProxyPage.test.tsx`（**顺序敏感的 `toEqual` 全数组断言**）、`McpPage.test.tsx`（**`toHaveLength(3)` 把 ZCode 扫描项算进去了，改 mock 后必须变 2**）需改。`hydrateApplyForm.test.ts` 0 命中（zcode 水合函数本来就没测）。仓库无快照测试。
9. 命令（未运行，属 implement/check 阶段）：`pnpm typecheck`（= `tsc --noEmit`）、`pnpm test:run`（= `vitest run`）。

---

## 1. 类型与联合

### `src/types/domain.ts`（14 命中）

| 位置 | 内容 | 移除动作 | 是否被编译强制 |
|---|---|---|---|
| 9–13 | 注释 + `export type ZCodeApiType = "anthropic-messages" \| "openai-responses" \| "openai-chat-completions"` | 整块删 | ❌ 不强制（导出符号） |
| **15** | `export type TargetKind = "claude_code" \| "codex" \| "pi" \| "prime" \| "zcode"` | 删 `\| "zcode"` | 源头改动 |
| 128–129 | `Site.zcodeApiType?: ZCodeApiType \| null` | 删（含注释） | ❌ 可选字段，不强制 |
| 174–175 | `CreateSiteInput.zcodeApiType?` | 删 | ❌ |
| 201–202 | `UpdateSiteInput.zcodeApiType?` | 删 | ❌ |
| 348–349 | `ApplyRequest.zcodeWriteAllModels?: boolean` | 删 | ❌（但 `applyStore.ts:111` 会因引用报错） |
| **379–380** | `AppSettings.zcodeHomeOverride: string \| null`（**必填、非可选**） | 删 | ✅ 强制 `settingsStore.ts:15` 的 `DEFAULT: AppSettings` 同步删（超额属性检查） |

其它 `TargetKind` 出现处（**无需改**，它们是数据驱动）：`271 TargetLiveStatus.kind`、`317 ApplyRequest.targets`、`353 ApplyTargetResult.target`、`398 AppSettings.localProxyTargets`、`599 ApplyRecord.target`、`610 BackupInfo.target`、`631 CliToolInfo.kind`。

穷举性说明：`domain.ts` 里 **没有** `Record<TargetKind, …>`，也没有 `TARGETS as const` 之类的常量数组；穷举发生在页面层（见下）。

### `src/types/mcp.ts:103`（1 命中）

```ts
/** 扫描目标：五个 Agent 客户端。 */
export type ScanTarget = "claude_code" | "codex" | "pi" | "prime" | "zcode";
```

**独立于 `TargetKind` 的第二份目标联合**（`ScannedMcp.target` / `ScanWarning.target` / `McpImportLocator.target` / `McpImportFailure.target` 都用它）。它镜像 Rust `src-tauri/src/adapters/mcp_scan.rs:32-38` 的 `enum ScanTarget { …, ZCode }`（`#[serde(rename_all = "camelCase")]` → 线上形态就是 `"zcode"`）。

关键耦合点：`McpPage.tsx:1107` 有 `entry.target as TargetKind`，1106 行注释明说「ScanTarget 与 TargetKind 取值一致，展示名可复用」。**如果只删 `TargetKind` 的成员而留着 `ScanTarget` 的成员，这个 `as` 转换在类型上不再成立（TS2352 会报「两种类型没有充分重叠」）**——这是一个隐性的「必须同时改」信号。

`src/types/domain.ts:21 SkillTarget` 与 `292 ThinkingTarget` **不含 zcode**（技能与思考预设从来不支持 ZCode）→ `SKILL_ROOTS: Record<SkillTarget,string>`（browserMock:300）无需改。`src/types/agent.ts:2` 的 `kind: string` 只是注释里列了 4 个目标，**无 zcode，无编译约束**（数据驱动：`agentUpdateStore.ts:55` 用 `Object.values`）。

### `src/stores/uiStore.ts:7`

```ts
export type ApplyTargetTab = "claude_code" | "codex" | "pi" | "prime" | "zcode";
```

第二份目标联合（应用中心左栏 tab）。穷举消费者：`ApplySidebar.tsx:15` `MENU_ICONS: Record<ApplyTargetTab, React.ReactNode>`。`applyTab` 初值是硬编码 `"claude_code"`（33 行），**不是 zcode，无需改初值**。

---

## 2. UI 组件面

### 面板挂载方式：**没有 target→panel 映射表**

`src/pages/ApplyPage.tsx` 是**手工展开的 5 段并列 JSX**（48–112 行），每段各自 `mounted.has("<id>")` + `display: applyTab === "<id>"` + `aria-hidden`，内容都是 `<AgentUpdateButton />` + `<XxxApplyPanel />`。zcode 是 **9 行 import + 100–112 行整块**。

含义：**删掉这一段不会有「index 错位」风险**（没有 `[0..4]` 下标、没有数组 `map` 出面板），但 `ApplyPage.tsx:9` 的未使用 import 会被 `noUnusedLocals` 拦下。

分发表实际存在于两处、且互不相同：
- `ApplySidebar.tsx:13` `const TAB_KEYS: ApplyTargetTab[] = […, "zcode"]`（顺序 = 菜单顺序）
- `ApplySidebar.tsx:15-21` `MENU_ICONS: Record<ApplyTargetTab, React.ReactNode>`，zcode 在 **20 行**

**图标来源确认**：ZCode 用 **lucide-react 的 `SquareTerminal`**（`ApplySidebar.tsx:5,20`，带 `data-icon="zcode"`），**不是 `@lobehub/icons`，也不是本地资源**（`public/` 只有 favicon；`find . -iname '*zcode*'` 在 `src/` 下只命中 2 个面板文件）。Prime 用 lucide `Sparkles`（与 agents.md 一致）。→ **`lucide-react` 依赖不能删**（其它地方还在用），但 `SquareTerminal` 这个具名 import 在 `ApplySidebar.tsx` 里**只服务 zcode**，删后会成为未使用 import。

### `src/components/apply/hydrateApplyForm.ts`（11 命中）的 zcode 边界

zcode 相关是**一段连续区间 110–139 行** + 顶部 2 个 import：

| 行 | 符号 | 备注 |
|---|---|---|
| 11 | `ZCodeApiType`（type import） | 随块删 |
| 9 | `SiteProtocol`（type import） | **只被 117 行 `inferZCodeApiType` 使用**（已 grep 确认全文件仅 9 与 117 两处）→ 删块后必须删这行 import |
| 110–114 | `const ZCODE_API_TYPES: readonly ZCodeApiType[]` | 仅被 `parseZCodeApiType` 用 |
| 116–119 | `export function inferZCodeApiType(protocol)` | 注释声明与后端 `ZCodeApiType::default_for` 一致 |
| 121–124 | `export function parseZCodeApiType(raw)` | 仅被 `hydrateZCodeForm` 用 |
| 126–128 | `export interface ZCodeFormDefaults extends PiFormDefaults` | |
| 130–139 | `export function hydrateZCodeForm(site, status)` | 内部复用 `hydratePiForm` |

**边界很干净**：`hydratePiForm`（254–270）与 `hydratePrimeForm`（272–277）不依赖任何 zcode 符号；zcode 块是单向消费者。

**但 `ZCodeApplyPanel.tsx:11–15` 从 `./hydrateApplyForm` 只 import 了 `buildModelOptions` / `hydrateZCodeForm` / `inferZCodeApiType`**——即 `parseZCodeApiType` 与 `ZCodeFormDefaults` 在面板外**没有其它消费者**。全部 zcode 导出符号在面板删除后都不再被引用，而 **`noUnusedLocals` 不检查 export**，因此这段是「静默死代码」，必须手工删。

### `src/components/apply/TargetStatusCard.tsx`（2 命中）

```ts
290 export function targetKindLabelKey(kind: TargetKind):
…
297   | "apply.targetZCode" {
298   if (kind === "claude_code") return "apply.targetClaude";
…
301   if (kind === "prime") return "apply.targetPrime";
302   return "apply.targetZCode";       // ← 默认兜底臂就是 ZCode
```

**这是本次最隐蔽的一处**：它不是 `if (kind === "zcode")` 显式臂，而是 **default 分支**。删掉 `'zcode'` 成员后这里**不会报任何编译错**（返回类型联合和 `if` 比较都不涉及 zcode 字面量），但：
- 消费者 `TargetStatusCard.tsx:80`（卡片标题）与 `SitesPage.tsx:308`（站点行的目标 chip）都在**渲染后端回传的 `status.kind`**，一旦后端仍返回 `kind:"zcode"`（v0.1.5 存量绑定还在），就落进这个默认臂 → 显示「ZCode」chip。删了 `apply.targetZCode` 键后 `t()` 找不到键会**渲染字面量 `apply.targetZCode`**。
- 建议改法：把默认臂换成显式的 `claude_code` 之类「已知值」，或加 `if (kind === "zcode") return …` 之前先删；实际最稳妥是**默认臂改为不受本次影响的键**并保留一个兜底文案。属于需要决策的点（见「风险」）。

### `src/components/apply/ApplyFooter.tsx`（7 命中）

zcode 出现在 4 处三元链，**全部是编译强制要改的**（`target: TargetKind` prop，12 行）：

- `23`：`isManagedProvider = target === "pi" || target === "prime" || target === "zcode"` → 去掉 `|| target === "zcode"`
- `29–30`（confirm title）：`target === "zcode" ? t("apply.removeZCodeConfirm") : …`
- `41–42`（confirm content）：`target === "zcode" ? t("apply.removeZCodeHint") : …`
- `63–64`（success content）：`target === "zcode" ? t("apply.removeZCodeDone") : …`

注意文案复用：`okText` / 成功标题 / 失败提示走的是 `apply.removePiOk`、`apply.removePiSuccess`、`apply.restoreOfficial*` 等**共享键**，删除 zcode 分支不影响它们。

### `src/components/apply/showApplyOutcome.tsx`（2 命中）

```ts
21 export function applyResultBodyKey(target: TargetKind):
…
28   | "apply.resultZCodeOk" {
32   if (target === "zcode") return "apply.resultZCodeOk";   // ← 删（编译强制）
33   return "apply.resultPiOk";                              // ← 默认臂是 Pi，不是 zcode
```

`28` 行的返回类型联合成员 `"apply.resultZCodeOk"` 也要删。与 `targetKindLabelKey` 不同，这里**兜底臂是 `resultPiOk`**，删掉 32 行后行为安全（未知/遗留 kind 会被说成 Pi，可接受度更高）。

### 移除后变成死分支/死代码的位置汇总

| 位置 | 状态 |
|---|---|
| `hydrateApplyForm.ts:110-139`（5 个导出符号） | **静默死代码**，编译不报错 |
| `domain.ts:13 ZCodeApiType` + 3 个 `zcodeApiType?` 可选字段 | 静默死类型/死字段 |
| `ApplyFooter.tsx` 3 条链上的 zcode 臂 | 编译强制删除 |
| `showApplyOutcome.tsx:32` | 编译强制删除 |
| `TargetStatusCard.tsx:302` 默认臂 | 编译不报错，但语义变成「未知目标一律标成 ZCode」 |
| `sites.goApplyZCode` i18n 键 | **本来就是死键**（`ef5c761` 之后） |

---

## 3. 状态 stores

### 结论：前端**没有任何持久化**，不存在脏键/hydrate 报错风险

- `grep -rn "persist\|createJSONStorage" src/stores/*.ts` → **0 命中**；`grep -rn "localStorage" src/` → 只有 `src/lib/routeProbe.ts:34,46,86`（路由探测缓存）与 `src/lib/siteIcon.ts:46,58,91`（站点图标缓存），都与 target 无关。
- 所以老用户机上唯一的 zcode 存量是 **SQLite**（`settings` JSON 的 `zcodeHomeOverride`、`sites.zcode_api_type`、`target_bindings` 的 zcode 行、`mcp_servers.targets` / `agent_rules.targets` / `localProxyTargets` 里的 `"zcode"`、`apply_records`/`backups` 历史行）—— 属 `backend-data-compat.md` 范围。

### `src/stores/applyStore.ts`（1 命中）

`111`：`zcodeWriteAllModels: req.zcodeWriteAllModels ?? false` —— `invoke("apply_site", {...})` 的**实参列表是手写的逐字段映射**（91–113 行）。删这一行即可；`TargetKind` 只在类型位置出现（29–33 行），无需改。

### `src/stores/settingsStore.ts`（1 命中）

`15`：`zcodeHomeOverride: null` —— `const DEFAULT: AppSettings`（5 行起）。因为 `AppSettings.zcodeHomeOverride` 是**必填**字段，删类型成员后这行必须同时删（否则超额属性报错）。

### `src/stores/uiStore.ts`（1 命中）

`7`：`ApplyTargetTab` 联合删成员。无持久化；`applyTab` 默认 `"claude_code"`。

### `src/stores/proxyStore.ts`（**0 命中**）

`setTakeover: (target: TargetKind, enabled)`（18/70 行）只有泛化的 `TargetKind`，**没有 zcode 字面量** → 不用改。但 `src/stores/proxyStore.test.ts:21` 有（见小节 7）。

---

## 4. 其余页面

| 页面 | 位置 | 形态 | 移除动作 |
|---|---|---|---|
| `src/pages/SettingsPage.tsx` | `477` `const [zcode, setZCode] = useState(settings.zcodeHomeOverride ?? "")`；`484`（`useEffect` 里的回填）；`490`（`useEffect` 依赖数组里的 `settings.zcodeHomeOverride`）；`499`（`onSave` 里 `zcodeHomeOverride: zcode.trim() \|\| null`）；**`533-541`** 整块 `<div className="mb-3">`（标题 `settings.zcodeHome` + `Input`（`settings.zcodeHomePlaceholder`）+ 说明 `settings.zcodeHomeHint`） | 「配置目录覆盖」输入框 + 保存字段 | 5 处 state/effect/依赖/保存 + 删 9 行 JSX 块。注意顺序：该卡片里块排在 `prime` 之后、`codex` 之前（533 起） |
| `src/pages/McpPage.tsx` | `52` `TARGETS: TargetKind[] = […, "zcode"]`；`59` `TARGET_LABEL_KEYS.zcode = "mcp.targetZCode"` | 目标**勾选列表** + 标签映射（两处 `Record` 与数组都是编译强制） | 删 52 行尾元素、59 行整行 |
| `src/pages/ProxyPage.tsx` | `10` `TARGETS`；`17` `TARGET_LABEL_KEYS.zcode = "proxy.targetZCode"` | 接管目标列表 | 同上 |
| `src/pages/RulesPage.tsx` | `10` `TARGETS`；`17` `TARGET_LABEL_KEYS.zcode = "rules.targetZCode"` | 全局约束勾选列表（138 行 `Checkbox.Group`） | 同上 |
| `src/components/apply/ApplySidebar.tsx` | `13` `TAB_KEYS`；`20` `MENU_ICONS.zcode`；`5` lucide `SquareTerminal` | 目标 tab + 图标 | 见小节 2 |
| `src/pages/ApplyPage.tsx` | `9` import；`100-112` 面板块 | | |
| `src/components/skills/*`、`ThinkingPreset*` | **0 命中** | 技能/思考预设本来就不含 ZCode | 无需改 |
| `src/pages/FloatingWindow*`、`Tray*` | **0 命中** | | |

**标签兜底不一致（值得顺手统一）**：`McpPage.tsx:265` 与 `RulesPage.tsx:59` 写成 `t(TARGET_LABEL_KEYS[target] ?? target)`（未知 target 退化成裸字符串 `"zcode"`）；而 `ProxyPage.tsx:168,169,360` 是 `t(TARGET_LABEL_KEYS[item.target])`，**没有 `??`** —— 若后端在删净之前仍把 `zcode` 塞进 `local_proxy_status.targets`，删除映射键后这里 `t(undefined)` 会抛错/渲染异常。见「风险 R3」。

**文档面（`src/` 之外，提示但不属前端代码）**：`README.md:13,32,87,224` 与 `README_EN.md:13,32,87,224` 各把 ZCode 列为受管目标（`README*.md:193` 说的是 `~/.zcode/tasks/` = **ZCode 这个 Agent CLI 自己的目录**，与产品目标无关，别一起删）。

---

## 5. i18n

**两个文件各 894 个 key，zcode-key 各 22 个，集合完全对称（zh-only = ∅，en-only = ∅）。** 25 次 grep 命中 = 22 个 key + 若干多行值/同值行，不是 25 个键。

### 22 个 key 路径（两文件同集）

```
apply.groupZCodeModels              apply.groupZCodeProtocol
apply.removeZCodeConfirm            apply.removeZCodeDone
apply.removeZCodeHint               apply.resultZCodeOk
apply.targetZCode                   apply.zcodeApiType
apply.zcodeApiTypeAnthropic         apply.zcodeApiTypeChatCompletions
apply.zcodeApiTypeHint              apply.zcodeApiTypeResponses
apply.zcodeFilesHint                apply.zcodeWriteAllModels
apply.zcodeWriteAllModelsHint
mcp.targetZCode      proxy.targetZCode      rules.targetZCode
settings.zcodeHome   settings.zcodeHomeHint settings.zcodeHomePlaceholder
sites.goApplyZCode
```

### 复用/共享文案（**按 key 名过滤会漏**）

- **`apply.dualWarning`** —— key 不含 zcode，但值里点名 ZCode，两种语言都被 `29284d7` 改过：
  - zh: `同一 model id 可能不被 Claude Code、Codex、Pi、Prime 与 ZCode 同时接受，仍可强制应用。`
  - en: `The same model id may not work across Claude Code, Codex, Pi, Prime, and ZCode. You can still force apply.`
  - **注意：`apply.dualWarning` 现在在 `src/` 里没有任何 `t()` 引用**（全仓 grep 只在两个 JSON 里出现）→ 要么删键，要么只改文案（去掉「与 ZCode」）后保留。这是需要决策的一项。
- 文案家族 `apply.removePi*` / `apply.removePrime*` 是**每个目标独立键**（`ApplyFooter` 按目标分支挑选），zcode 用的是自己独立的一套 `removeZCode*`，所以删 zcode 不会伤到 Pi/Prime 文案。
- 顺带（**非** zcode，但同类「文案里枚举目标」的陈旧问题，本次可不动）：`mcp.emptyDesc`、`mcp.existingDesc`、`sites.emptyDesc`、`sites.keySwitchHint` 只列了 `Claude Code、Codex、Pi、Prime`——**它们本来就没提 ZCode**，删掉 ZCode 后反而变正确，无需改。

### 死键清单（脚本核对：22 个里 21 个有代码引用，1 个没有）

- `sites.goApplyZCode` —— 死键。且**同族 `sites.goApplyClaude` / `sites.goApplyCodex` / `sites.goApplyPi` / `sites.goApplyPrime` 也都是死键**（`ef5c761` 把按钮改成固定 `t("sites.goApply")`，`GoApplyButton.tsx:25`）。zcode 那个必须删；同族其余是既有债，可选清理。
- 动态拼 key 的 `t()` 只有 4 种模式（`errors.${code}`、`apply.status_${status}`、`apply.thinkingLevel_${level}`、`settings.${key}`），**没有任何一种按目标名拼 key** → 上面的「静态引用扫描」结论可信。

---

## 6. 浏览器 mock 契约（`src/lib/browserMock.ts`，25 命中）

agents.md 要求「invoke 契约须与 Rust command 保持同步」。逐位点与真实 command 对齐如下（Rust 侧只读签名/serde，不分析业务）：

| browserMock 位置 | 内容 | 对应真实 Rust 侧 | 移除动作 |
|---|---|---|---|
| `64` | `DEFAULT_SETTINGS.zcodeHomeOverride: null` | `src-tauri/src/domain/mod.rs:573 AppSettings`（`#[serde(rename_all="camelCase")]`）`:587 zcode_home_override: Option<String>` | 与 `settingsStore.ts:15` 同批删 |
| `156-170` | `defaultTargetStatuses()` 里整条 `kind:"zcode"` 记录（`configPath: "~/.zcode/v2/config.json"`） | `list_target_status`；Rust 硬编码遍历 5 元数组 `commands/targets.rs:41-47`（含 `TargetKind::ZCode`） | **整条删** |
| `185` | `mockProxyStatus()` 的 `["claude_code","codex","pi","prime","zcode"] as TargetKind[]` | `local_proxy_status` 的 `targets` 数组 | 删数组尾元素（188 行的 `/v1` 后缀三元里没有 zcode，无需改） |
| `274-284` | `INITIAL_SCANNED_MCP` 里 `target:"zcode", key/name:"existing-zcode"` 条目 | `scan_existing_mcp` ← Rust `enum ScanTarget { …, ZCode }`（`adapters/mcp_scan.rs:32-38`） | **整条删**（否则 `McpPage.test.tsx:446` 与 `:471` 的计数会错） |
| `295` | `AGENT_RULES_PATHS` 的 `["zcode","/Users/demo/.zcode/AGENTS.md",false]` | `agent_rules_target_paths` | 删该元组 |
| `942` | `create_site` 里 `zcodeApiType: input.zcodeApiType \|\| null` | `CreateSiteInput.zcode_api_type`（`domain/mod.rs:218`） | 删（配合类型） |
| `1068-1071` | `update_site` 里 `zcodeApiType: input.zcodeApiType !== undefined ? … : (s.zcodeApiType ?? null)` | `UpdateSiteInput.zcode_api_type`（`domain/mod.rs:288`） | 删整个键值对 |
| `1295-1300` | `apply_site`：`zcodeWriteAllModels` / `zcodeModelCount` / `zcodeApiType` 三个局部量 | `commands/apply.rs:64 zcode_write_all_models: Option<bool>` | **整段删** |
| `1310` | `row.kind === "pi" \|\| "prime" \|\| "zcode"` 决定 `providerId = xiaobai_…` | 同上 | 去掉 `\|\| row.kind === "zcode"` |
| `1328-1335` | `apply_site` 的 zcode `liveSummary` 分支（`apiType` / `modelCount` / `writeAllModels`） | 同上 | 删该三元臂 |
| `1540` | `detect_cli_tools` 返回里的 `{ kind:"zcode", … }` | `detect_cli_tools` | 删该元素 |
| `2142` | `mcp_target_paths` 返回里的 `["zcode","/Users/demo/.zcode/cli/config.json"]` | `mcp_target_paths`（Rust 侧 `adapters/mcp.rs:413 zcode_home_override`） | 删该元组 |

**mock 侧命令名共 106 个 case，本次涉及 12 个位点**。`resetBrowserMock()`（400 行起）会 `targetStatuses = defaultTargetStatuses()` 并重置 `scannedMcp`，所以**测试断言直接来自这些常量**（见小节 7 的三处顺序/计数敏感测试）。

**契约同步顺序（重要）**：
- `save_settings` 走 `partial: serde_json::Value` + `repo/settings.rs:57 preview_merge`（把 partial 的任意键 `insert` 进 JSON 再 `from_value::<AppSettings>`），全仓**没有 `deny_unknown_fields`**（只有 `sync.rs:750` 的一句注释明确依赖「未知字段必须被忽略」）。→ **前端多传 `zcodeHomeOverride` 给已删除该字段的后端会被静默忽略**，不会报错；旧 DB 行里残留该键同理。这降低了「前后端必须原子发布」的压力。
- 但 **`TargetKind` / `ScanTarget` 的 serde 枚举是硬边界**：如果后端 `TargetKind` 删了 `ZCode` 而数据库仍有 `target='zcode'` 的行，`from_value`/`from_str` 会**直接失败**；反之后端未删而前端类型先删，只是 UI 不展示。**这个方向的核对属于 `backend-data-compat.md`**，但前端必须知道：`list_target_status` 返回条目的 `kind` 值集合决定了 `SitesPage` chip 与 `TargetStatusCard` 标题会不会冒出 ZCode。

---

## 7. 测试面

跑法（**本次未执行**）：`pnpm typecheck`、`pnpm test:run`（agents.md 规定：前端改动未跑检查不得称「已完成」）。

| 测试文件 | 断言的是什么 | 移除后动作 |
|---|---|---|
| `src/components/apply/ZCodeApplyPanel.test.tsx`（157 行，13 命中） | ① 回填站点已存协议（`openai-chat-completions` 而不是按站点 protocol 推断的 `openai-responses`）+ 由 live summary 还原「写入全部模型」；② 改协议 → **先 `updateSite(id,{zcodeApiType})` 落库、后 `apply({targets:["zcode"], zcodeWriteAllModels:true})`**，并用 `invocationCallOrder` 断言这个顺序 | **整个文件删除**（面板不存在） |
| `src/components/apply/ApplySidebar.test.tsx:60-72` | 「渲染 ZCode tab、有 `[data-icon="zcode"]`、点击后 `applyTab === "zcode"`」 | **删这 1 个 `it`**（60–72 行）。前 2 个用例（Pi/Prime）保留。注：32 行用例名「renders the **third** Pi tab」里的序数在 ZCode 存在时本就没错，删后仍对 |
| `src/components/apply/showApplyOutcome.test.ts:18` | `applyResultBodyKey("zcode") === "apply.resultZCodeOk"` | **删这一行**（`"zcode"` 实参在类型收窄后会 TS 报错） |
| `src/stores/proxyStore.test.ts:16-22` | `status.targets.map(t=>t.target)` **有序全等** `[claude_code, codex, pi, prime, zcode]` | **改为 4 元素数组**（删 `21` 行的 `"zcode"`）。顺序敏感：数据来自 `mockProxyStatus()` 的数组顺序 |
| `src/pages/ProxyPage.test.tsx:40,50,57` | 用例名「shows all **five** takeover targets…」+ 同样的有序全等数组 + `getByText("ZCode")` | 删 `50` 行元素、删 `57` 行断言、**把用例名里的 five 改成 four**（否则名字说谎） |
| `src/pages/McpPage.test.tsx:446,470-471` | 「打开即扫描」列表里出现 `existing-zcode`；`:471` `getAllByRole("button",{name:/^纳\s*管$/})` **`toHaveLength(3)`**，470 行注释明写「三条可纳管（**含 ZCode 扫描项**）」 | 删 `446` 行；`471` 的 `3` **改成 `2`**；470 行注释同步改写 |
| `src/components/apply/hydrateApplyForm.test.ts` | **0 命中** —— `hydrateZCodeForm` / `parseZCodeApiType` / `inferZCodeApiType` 从来没有单测 | 无需改；也意味着**删这些函数没有任何测试保护网会响**（只能靠 typecheck + 人工核对） |
| 其它相关但 **0 命中** 的文件 | `ApplyPage.test.tsx`（用 `getByRole("menuitem",{name:"Codex"})` 按名字取，无面板数量断言）、`ApplyFooter.test.tsx`（断言「已还原官方配置」文案）、`RulesPage.test.tsx:73`（`getAllByText(/已写入/).length === 2` 是**已写入结果条数**，与目标数量无关）、`SettingsPage.test.tsx:426-458`（paths 段用 `findByPlaceholderText` 寻址，**不按 textbox 下标**）→ 删 ZCode 输入框不会错位 | 无需改 |
| 快照测试 | 全仓 `toMatchSnapshot` / `__snapshots__` / `*.snap` → **0 命中** | 无 |

---

## 8. 穷举式 / 隐式依赖「目标数量·顺序」的引用扫描

| # | 位置 | 类型 | 结论 |
|---|---|---|---|
| I1 | `TargetStatusCard.tsx:298-302` `targetKindLabelKey` | **默认兜底臂 = `"apply.targetZCode"`** | ⚠️ **静默**：删成员后不报错，未知 kind 一律显示 ZCode；删 i18n 键后渲染裸 key 字符串。必须显式改写 |
| I2 | `showApplyOutcome.tsx:29-33` `applyResultBodyKey` | 显式 zcode 臂 + 默认臂 `resultPiOk` | 臂本身编译强制删；默认臂安全，无需改 |
| I3 | `proxyStore.test.ts:16-22`、`ProxyPage.test.tsx:45-51` | `toEqual([...])` **有序全等** | ⚠️ 与 mock 数组顺序锁死，必须同批改 |
| I4 | `McpPage.test.tsx:471` `toHaveLength(3)` | **数量断言含 zcode 项** | ⚠️ 改 mock 数据后必须 3→2；这是唯一「数字直接算进目标个数」的断言 |
| I5 | `ApplyPage.tsx:48-112` | 5 段并列 JSX（**非**数组/索引） | 安全：删一段不影响其它面板寻址 |
| I6 | `browserMock.ts:1262,2085,2148`、`RulesPage.tsx:138`、`McpPage.tsx:344` | `as TargetKind[]` 后 `.filter/.includes` 数据驱动 | 安全（无下标） |
| I7 | `hydrateApplyForm.ts:49` `sites[0]?.id`、`ModelPicker.tsx:158` `models[0]`、`TargetBackupList.tsx:21` `names[0]`、`BackupQuickPopover.tsx:19` `items[0]` | 下标访问但对象是站点/模型/备份，**不是目标** | 无关 |
| I8 | `src/lib/packagingWorkflow.ts:1-7` `REQUIRED_RELEASE_TARGETS` | 名字带 target 但是 **Rust 构建 triple**（`aarch64-apple-darwin` …） | **假阳性关键词，绝对不要动** |
| I9 | `McpPage.tsx:1107` `entry.target as TargetKind` | 跨联合（`ScanTarget`→`TargetKind`）的**强制断言** | ⚠️ 两个联合必须**同批**删成员，否则类型转换报错；注释 1106 行声明的「取值一致」不变式是隐式契约 |
| I10 | `ApplySidebar.tsx:13` `TAB_KEYS` 顺序 | 决定左栏菜单顺序；无按 index 取用（`MENU_ICONS` 是按键） | 安全 |
| I11 | `useDeferredTabContent(applyTab)` 的 `mounted: Set<ApplyTargetTab>` | 泛型、按 `active` 增量挂载 | 安全 |
| I12 | 无持久化（见小节 3） | — | 无脏键风险；`applyTab` 无遗留 `"zcode"` 可能（内存态，重启即默认） |

`grep` 完整性交叉验证：`grep '"prime"' src/`（prime 是第 4 个目标，任何「列出所有目标」的代码必然出现它）得到 40 处，与已覆盖文件集合一致：`ApplyFooter/ApplySidebar/McpPage/ProxyPage/RulesPage/uiStore/domain/mcp/ApplyPage/hydrateApplyForm/showApplyOutcome/TargetStatusCard/thinkingPreset/PrimeApplyPanel/SkillTargetIcon/agent.ts`。**其中 `types/agent.ts:2`（注释枚举）、`thinkingPreset.ts:46`（`ThinkingTarget` 只含 pi/prime）、`SkillTargetIcon.tsx:8`（`SKILL_TARGETS` 不含 zcode）三处是「本来就没接 ZCode」的位置，无需改**——这也反向确认了 zcode 的接入面没有隐藏文件。

---


---

> 本文件的后半部分（自 `## 移除清单` 起）已拆分到 [`frontend-removal-worklist.md`](./frontend-removal-worklist.md)，以免超出 context injection 的 32768 字节上限被截断。两份必须一起读。
