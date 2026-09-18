# 实施计划：添加站点预置模板（opencode-go / 魔搭）

按 M1 → M2 顺序执行；每个阶段结束是一个可独立提交的检查点。测试先写、先看它红。

## M1 前端模板与凭据免除

- [x] 1. 新建 `src/lib/siteProviderKinds.ts`：`isOpenCodeGoBase` / `isModelScopeBase` / `sitePresetFor` / `quotaCredentialHint`。
  - 先写测试 `src/lib/siteProviderKinds.test.ts`，opencode 用例从 `src-tauri/src/quota_probe/mod.rs:3771-3785` 的反例照搬（`api.opencode.ai`、`opencode.ai.evil.com`、`http://`、`/zen/gopher`、`/zen/go-v2`），魔搭补 `modelscope.cn`、`api-inference.modelscope.cn.evil.com`、`http://api-inference.modelscope.cn`。
  - 验证：`npx vitest run src/lib/siteProviderKinds.test.ts`
- [x] 2. `src/lib/browserMock.ts:672-685` 的平行实现改为 import 第 1 步的 helper，删掉本地副本。
  - 验证：`npx vitest run src/lib src/pages/FloatingWindow.test.tsx`
- [x] 3. 新建 `src/lib/sitePresets.ts` 目录常量（`custom` / `opencode-go` / `modelscope`），字段见 design.md 第 3 节；文案 key 落 `src/i18n/locales/zh-CN.json` 与 `en-US.json`。
- [x] 4. 「添加站点」入口接模板选择：只走 `SiteFormInitialValues` + `SitesPage.tsx:89-102` 既有通路，不新建第二个创建弹窗、不扩 `SiteFormInitialValues` 字段。
  - 测试：选魔搭后表单里 Base URL / 协议已预填、保存只需名字 + key。
- [x] 5. `SiteFormModal.tsx:612-631` 的额度组按 `sitePresetFor(当前第一项 Base URL)` 免除；判断只看输入，不看点击历史。
  - 测试：改成第三方 host 后免除提示消失。
- [x] 6. 阶段检查点：`pnpm typecheck && pnpm test:run`。

## M2 魔搭魔粒余额

- [x] 7. Rust：`is_modelscope_base` + `QuotaSource::MagicubeBalance`（`domain/mod.rs:352-363`、TS 镜像 `types/domain.ts:540-548`）。
  - `quota_source_rank` 是穷尽匹配，新枚举值必须补一条；专用渠道永远不进那个合并，注释已写明。
- [x] 8. Rust：`probe_quota` 早退分支（插在 `quota_probe/mod.rs:1704-1724` 之间）+ 响应解析，状态映射按 design.md 第 4 节表格。
  - 表驱动单测：正常 / 缺 `data` / 非数值 / `success:false` / 401 / 5xx / 超时。断言不出现 0 余额兜底。
  - 「超时」用**关掉监听端口**的传输失败替代（同一 `Err` 分支，只差 `is_timeout()` 的文案），避免 8 秒超时拖慢单测。
  - 变异验证：host 判据改 `ends_with` → 只 `modelscope_base_detection` 红；去掉 403 → 只两条状态映射测试红；缺失余额改 `unwrap_or_default()`（伪造 0）→ 只映射表 + 探测红。三次都精确命中后还原。
- [x] 9. Rust：`commands/quota.rs:83-85` 空 key 放行条件扩成两个已知 host。
  - 抽成 `quota_probe::allows_empty_key_probe` 以便单测；测试：空 key 魔搭站点返回 `unauthorized` 而不是 `unsupported`，且不发请求。
- [x] 10. 脱敏检查：新增错误路径的 `error` 文本只含状态码与 `code`；`cargo test` 里加一条断言错误体不含 Bearer 值。
  - `code` 过滤到 `[A-Za-z0-9_.-]` 并截 64 字符（`magicube_quota_truncates_a_runaway_code_field`）；`message` 一律不进错误串。
- [x] 11. 前端：`normalizeQuotaUnit` + `formatQuotaAmountParts` 加 `MAGICUBE` 分支、`sites.quotaUnitMagicube` 双语；**不要**碰 `quotaProbe.ts:90-95` 的 `RAW_QUOTA` 字面量分支。
  - 测试：`1000.5 MAGICUBE → "1,000.5 魔粒"`、en → `"1,000.5 Magicubes"`、无 `$`/`¥`。
  - **偏离原计划**：`formatQuotaAmount` 那条 `RAW_QUOTA` 字面量分支必须动。它假设「有 `unitI18nKey` 就是点数」，第二个 key 一进来魔粒金额就会被印成 `RAW_QUOTA`（比原样打印更错）。改成 `QUOTA_UNIT_FALLBACKS[key]` 查表，`RAW_QUOTA` 的输出逐字不变并有测试钉住。
- [x] 12. `browserMock` 补魔搭 balance 分支，形状与 Rust 对齐。
  - 位置同 Rust：opencode 分支之后、通用 `!hasKey → unsupported` 之前；空 key 给 `unauthorized`。
- [x] 13. 悬浮窗：确认 `refresh_site_quota` 那一腿自动带上魔搭（同一 `probe_quota_for`），不新增事件与定时器。
  - 数据那一腿确实自动继承（`commands/floating.rs:74-78` → `probe_quota_for`，无新事件/定时器）。**但显示不继承**：`FloatingWindow.tsx` 自带 `formatQuota` 把金额硬编码成 `` `$${n.toFixed(2)}` ``，魔粒会渲染成 `$1000.50`，直接违反 prd「看不到任何 `$`/`¥`」的验收；顺带 `RAW_QUOTA` 与 `CNY` 站点一直也被印成美元。改为走 `formatQuotaAmountLocalized(n, quota.unit, t)`，测试 `lists site balances in the five display shapes` 补一条 `1,000.5 魔粒`。

## 收尾

- [x] 14. 全量验证：`pnpm typecheck` 干净、`pnpm test:run` 435 passed（只有既存 shebang 两套红）、`src-tauri` 下 `cargo test` 567 passed / 0 failed。
- [ ] 15. 真机确认（不能只靠单测声称完成）：装包后用真实魔搭令牌走一遍模板，确认余额显示与 401/无效令牌两种态；无凭据时明确说明未做真机验证。
  - **未做**：本机没有可用的 ModelScope API Token，也没有跑起打包后的应用。Rust 侧只有本地 mock server 的响应形状；真实字段名、`success`/`code` 取值、限流行为都未经真机核对。
- [x] 16. `.trellis/spec` 更新：`frontend/state-management.md` 补「服务商识别单一来源」契约；跨层条目补 `QuotaSource` / `unit` 加值的双侧同步要求。
  - 另加一条通用口径：额度金额只能经 `formatQuotaAmountLocalized` 出口（悬浮窗那处硬编码 `$` 就是这么来的）。

## 已知无关失败

`generateUpdaterManifest.test.ts`、`validateUpdaterSigningSecret.test.ts` 因 shebang 解析失败，先前即存在，不视为本任务回归。
