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

- [ ] 7. Rust：`is_modelscope_base` + `QuotaSource::MagicubeBalance`（`domain/mod.rs:352-363`、TS 镜像 `types/domain.ts:540-548`）。
- [ ] 8. Rust：`probe_quota` 早退分支（插在 `quota_probe/mod.rs:1704-1724` 之间）+ 响应解析，状态映射按 design.md 第 4 节表格。
  - 表驱动单测：正常 / 缺 `data` / 非数值 / `success:false` / 401 / 5xx / 超时。断言不出现 0 余额兜底。
- [ ] 9. Rust：`commands/quota.rs:83-85` 空 key 放行条件扩成两个已知 host。
  - 测试：空 key 魔搭站点返回 `unauthorized` 而不是 `unsupported`。
- [ ] 10. 脱敏检查：新增错误路径的 `error` 文本只含状态码与 `code`；`cargo test` 里加一条断言错误体不含 Bearer 值。
- [ ] 11. 前端：`normalizeQuotaUnit` + `formatQuotaAmountParts` 加 `MAGICUBE` 分支、`sites.quotaUnitMagicube` 双语；**不要**碰 `quotaProbe.ts:90-95` 的 `RAW_QUOTA` 字面量分支。
  - 测试：`1000.5 MAGICUBE → "1,000.5 魔粒"`、en → `"1,000.5 Magicubes"`、无 `$`/`¥`。
- [ ] 12. `browserMock` 补魔搭 balance 分支，形状与 Rust 对齐。
- [ ] 13. 悬浮窗：确认 `refresh_site_quota` 那一腿自动带上魔搭（同一 `probe_quota_for`），不新增事件与定时器。

## 收尾

- [ ] 14. 全量验证：`pnpm typecheck`、`pnpm test:run`、`src-tauri` 下 `cargo test`。
- [ ] 15. 真机确认（不能只靠单测声称完成）：装包后用真实魔搭令牌走一遍模板，确认余额显示与 401/无效令牌两种态；无凭据时明确说明未做真机验证。
- [ ] 16. `.trellis/spec` 更新：`frontend/state-management.md` 补「服务商识别单一来源」契约；跨层条目补 `QuotaSource` / `unit` 加值的双侧同步要求。

## 已知无关失败

`generateUpdaterManifest.test.ts`、`validateUpdaterSigningSecret.test.ts` 因 shebang 解析失败，先前即存在，不视为本任务回归。
