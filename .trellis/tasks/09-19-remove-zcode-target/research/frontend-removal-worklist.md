## 移除清单（可勾选，按文件组织）

### A. 类型（穷举源头 —— 建议最先改，让 `tsc` 帮你找其余）
- [ ] `src/types/domain.ts`
  - [ ] 删 9–13（`ZCodeApiType` 及其注释）
  - [ ] 改 15：`TargetKind` 去掉 `| "zcode"`
  - [ ] 删 128–129 / 174–175 / 201–202（3 个 `zcodeApiType?`）
  - [ ] 删 348–349（`ApplyRequest.zcodeWriteAllModels`）
  - [ ] 删 379–380（`AppSettings.zcodeHomeOverride`）
- [ ] `src/types/mcp.ts:102-103` —— `ScanTarget` 去掉 `"zcode"`，并把注释「五个 Agent 客户端」改为「四个」
- [ ] `src/stores/uiStore.ts:7` —— `ApplyTargetTab` 去掉 `"zcode"`

### B. 应用中心
- [ ] `src/pages/ApplyPage.tsx` —— 删 9（import）、100–112（整块）
- [ ] `src/components/apply/ApplySidebar.tsx` —— 删 13 尾元素、19→20 行的 `zcode:` 项、5 行 import 的 `SquareTerminal`（只留 `Sparkles`）
- [ ] `src/components/apply/ZCodeApplyPanel.tsx` —— **整文件删除**
- [ ] `src/components/apply/hydrateApplyForm.ts` —— 删 11（`ZCodeApiType`）、**9（`SiteProtocol`，删后未被使用）**、110–139（`ZCODE_API_TYPES`/`inferZCodeApiType`/`parseZCodeApiType`/`ZCodeFormDefaults`/`hydrateZCodeForm` 整段）
- [ ] `src/components/apply/TargetStatusCard.tsx` —— 删 297 的 `| "apply.targetZCode"`；**改写 302 的默认臂**（不能再指向 ZCode 文案；建议 `if (kind === "zcode")` 之类彻底不存在后，把默认臂设为某个仍存在的键或显式 4 分支 + 抛错/回退）
- [ ] `src/components/apply/showApplyOutcome.tsx` —— 删 28 的联合成员、32 的臂
- [ ] `src/components/apply/ApplyFooter.tsx` —— 改 23；删 29–30、41–42、63–64 三处臂（注意保持三元链括号/缩进闭合）

### C. stores / 页面
- [ ] `src/stores/applyStore.ts:111` —— 删 `zcodeWriteAllModels` 实参行
- [ ] `src/stores/settingsStore.ts:15` —— 删 `zcodeHomeOverride: null`
- [ ] `src/pages/SettingsPage.tsx` —— 删 477、484、490（依赖数组元素）、499、533–541（整块）
- [ ] `src/pages/McpPage.tsx` —— 删 52 尾元素、59 整行
- [ ] `src/pages/ProxyPage.tsx` —— 删 10 尾元素、17 整行（并考虑给 168/169/360 补 `?? target` 兜底，见 R3）
- [ ] `src/pages/RulesPage.tsx` —— 删 10 尾元素、17 整行

### D. browserMock（与 Rust 同批）
- [ ] `src/lib/browserMock.ts` —— 64 / 156–170 / 185 / 274–284 / 295 / 942 / 1068–1071 / 1295–1300 / 1310 / 1328–1335 / 1540 / 2142（共 12 位点）

### E. i18n（两文件同集各删 22 键）
- [ ] `src/i18n/locales/zh-CN.json` —— 上表 22 个 key（`apply.*` 15 个 + `mcp.targetZCode` + `proxy.targetZCode` + `rules.targetZCode` + `settings.zcodeHome{,,Hint,Placeholder}` 3 个 + `sites.goApplyZCode`）
- [ ] `src/i18n/locales/en-US.json` —— 同集 22 个 key
- [ ] `apply.dualWarning`（**两个文件**）—— 决策：删键（已是死键）或改文案去掉「与 ZCode」/「, and ZCode」
- [ ] （可选债）同族死键 `sites.goApplyClaude/Codex/Pi/Prime` 一并清

### F. 测试
- [ ] 删 `src/components/apply/ZCodeApplyPanel.test.tsx`（整文件）
- [ ] `src/components/apply/ApplySidebar.test.tsx` —— 删 60–72
- [ ] `src/components/apply/showApplyOutcome.test.ts` —— 删 18
- [ ] `src/stores/proxyStore.test.ts` —— 删 21（5→4 元素）
- [ ] `src/pages/ProxyPage.test.tsx` —— 删 50、57；40 行用例名 five→four
- [ ] `src/pages/McpPage.test.tsx` —— 删 446；471 `3`→`2`；470 注释改写

### G. 文档（非 `src/`，建议同批）
- [ ] `README.md` / `README_EN.md` 的 13、32、87、224 行（**保留 193 行**，那是 ZCode CLI 自己的 `~/.zcode/tasks/`）

### H. 不要动（关键词撞车 / 与产品目标无关）
- [ ] 仓库根 `.zcode/` 与 `.trellis/.backup-*/.zcode`（Trellis 的 **ZCode 平台适配目录**，见 `.claude/skills/trellis-meta/references/platform-files/agents.md`）
- [ ] `src/lib/packagingWorkflow.ts:1-7`（Rust 构建 triple）
- [ ] `lucide-react` / `@lobehub/icons` 依赖（无变化）
- [ ] `.trellis/tasks/archive/2026-09/09-15-zcode-target/**`（历史任务归档，是记录不是代码）

---

## 风险与未决问题

### 确证

- **R1（高）｜死代码不会报错**：`hydrateApplyForm.ts:110-139` 五个**导出**符号 + `domain.ts` 的 `ZCodeApiType`/3 个可选字段在删面板后**完全静默**（`noUnusedLocals` 不覆盖 export）。编译与测试都通过 → 必须靠本清单人工核对。
- **R2（高）｜`targetKindLabelKey` 默认臂**（`TargetStatusCard.tsx:302`）：只要后端 `list_target_status` 仍回 `kind:"zcode"`（v0.1.5 存量绑定），`SitesPage.tsx:301-308` 的 chip 与 `TargetStatusCard.tsx:80` 的卡片标题就会冒出「ZCode」；删了 i18n 键后退化成裸 `apply.targetZCode` 字符串。**这是前后端唯一的「用户可见残留」通道**，与 backend agent 的「是否让 `list_target_status` 停止返回 zcode」决策直接相关。
- **R3（中）｜`ProxyPage` 无 `?? target` 兜底**（168/169/360 行）：`local_proxy_status.targets`（Rust 生成）若仍含 `zcode`，删映射键后 `t(undefined)` 行为未定义（i18next 会告警并返回 undefined/裸 key）。建议顺手补 `?? target`（与 McpPage/RulesPage 对齐）。
- **R4（中）｜`ScanTarget` 与 `TargetKind` 的「取值一致」隐式不变式**（`McpPage.tsx:1106-1107` 的 `as` 转换 + 注释）。两个联合必须同批改；只改一个会 TS 报错（好事）或留下不成立的断言。
- **R5（中）｜`apply_site` 实参形状**：`applyStore.ts:111` 与 Rust `commands/apply.rs:64` 是**逐字段手写映射**。删一侧、留另一侧都不报错（前端多传 → Tauri 忽略；少传 `Option<bool>` → None）。风险是「契约漂移」而不是崩溃。
- **R6（低）｜顺序/计数测试**：小节 7 表格里 3 处（`proxyStore.test`、`ProxyPage.test`、`McpPage.test` 的 `toHaveLength(3)`）。它们**会失败**（不会静默），但失败信息可能不直观，尤其 `McpPage.test.tsx:471` 的 3→2。

### 推测（附验证方法）

- **P1**：`AppSettings.zcodeHomeOverride` 在 TS 里是必填、在 Rust 里是 `Option<String>`，我推断「缺字段可反序列化为 None」，依据是 `repo/settings.rs` 既有用例 `old_settings_json_gets_proxy_defaults`（第 91 行插入的 JSON 完全没有 `zcodeHomeOverride`/`piAgentDirOverride` 却能 `get_settings` 成功）。**验证**：implement 阶段跑 `cargo test` 里 settings 相关用例，或读 `domain/mod.rs:587` 是否带 `#[serde(default)]`。
- **P2**：我未逐一验证 Tauri 2 对「command 多收/少收参数」的处理（前端多传 `zcodeWriteAllModels`、Rust 已删参）。**验证**：`cargo run` 浏览器模式之外做一次真机应用，或直接看 `#[tauri::command]` 生成的 arg 解析；也可先按「同批删」执行以规避。
- **P3**：`i18next` 的 `t(undefined)` 具体行为（抛错 vs 返回 undefined vs 打警告）未实测。**验证**：`pnpm test:run` 里给 ProxyPage 造一个含 zcode 的 `status.targets` 跑一次；或按 R3 加兜底后即无意义。
- **P4**：`tray.rs` / `tray_apply.rs` / `key_switch.rs` 等 Rust 侧有各自的 5 目标枚举（`grep` 显示 `tray.rs` 32 次命中），**托盘菜单是否会显示 ZCode 不由前端控制** —— 归 `backend-data-compat.md`。
- **P5**：`apply.dualWarning` 与 `sites.goApply*` 死键的处置（删 vs 改）属产品决策，非技术问题。

### 未决问题（需主 agent/用户定）

1. `TargetStatusCard.targetKindLabelKey` 的默认臂改成什么？（选项：指向 `apply.targetPrime`／引入一个 `apply.targetUnknown` 新键／改成显式 4 分支 + 兜底 `kind` 原样）
2. `apply.dualWarning` 是删（连带其它死键）还是仅去 ZCode 文案？
3. 是否在本任务里同批删 `README*` 的 ZCode 文案（PRD 只写了「前端面板与文案」）。
