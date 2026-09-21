# 站点列表额度缺失原因改短标签

## Background

上一轮（原型 `.trellis/prototypes/site-quota-reason-placement.html`）给了三个摆放方案，用户选 **C**：

- A 保持现状：列表行与详情面板都显示完整句子 —— 同一句话一屏两遍，且 232px 列表下被截断。
- B 选中行留空：消除重复，但把「没额度就空白」这个原始 complaint 放回去。
- **C 采用**：列表行只放四字短标签，完整句子只留在详情面板，两处形成「摘要 → 说明」而不是两份副本。

数据层（`quotaBySite` 只存成功结果、`quotaAttemptBySite` 存最近一次尝试、`errorQuotaAttempt()` 合成失败记录）已在上一轮完成，本任务只改展示文案，不动数据流。

## Requirements

- R1. 列表行第二行的额度缺失原因改用**短标签**，覆盖现有 4 个探测状态：
  - `unsupported` → 「无额度接口」
  - `unauthorized` → 「鉴权失败」
  - `invalid_data` → 「数据异常」
  - `error` → 「获取失败」
- R2. 新增独立的 i18n key（`sites.quotaLabel*`），**不复用也不改写**详情面板使用的 `sites.quotaUnsupported` 等全称 key；中英文两份都要有，组件内不硬编码中文。
- R3. 详情面板 `SiteQuotaRow` 的展示与文案保持逐字节不变，包括超时 / 上游 5xx 的细分分支。
- R4. 选中行与未选中行使用同一套短标签，选中不改变第二行内容。
- R5. 没有额度尝试记录时第二行仍然完全留空（区分「没探测过」与「探测过但没额度」）。
- R6. 未知 status 值不渲染任何标签，保持现有兜底行为。
- **R7. 列表行补两个 `available` 状态的兜底分支**（修复 AnyRouter 等站点空白问题）：
  - `available` 且 `unlimited: true` → 显示「无限额度」（与详情面板 `SiteQuotaRow` 文案一致，走现有 i18n key `sites.quotaUnlimited`）
  - `available` 且 `remainingUsd == null` 且无 `windows` → 显示「额度未知」（与详情面板一致，走现有 key `sites.quotaUnknown`）
- R8. 两个兜底分支的文字颜色用 `token.colorTextQuaternary`（与失败状态标签同色，区分于正常余额的 `colorTextTertiary`）。

## Non-Goals

- 不改并发上限、刷新调度、超时策略。
- 不引入魔搭 / opencode-go 的额度来源改动，那属于 `09-18-site-add-presets`。
- 不改列表列宽或行高。

## Acceptance Criteria

- [ ] 同一屏内不再出现额度缺失原因的完整句子重复：列表行只有短标签，详情面板只有全称。
- [ ] 4 个失败状态各自的短标签在列表行正确渲染；未知状态不渲染。
- [ ] 2 个 `available` 兜底分支（无限额度 / 额度未知）在列表行正确渲染，颜色为 `colorTextQuaternary`。
- [ ] 未探测过的站点第二行仍为空白，不显示任何标签。
- [ ] `SitesPage.test.tsx` 中「同句出现两处」的断言被替换为「短标签在列表 + 全称在详情」各一处。
- [ ] `SiteListItem.test.tsx` 的占位断言改用短标签文案；新增 unlimited / remainingUsd=null 的渲染断言。
- [ ] `pnpm typecheck` 与 `pnpm test:run` 通过（已知无关失败：`generateUpdaterManifest.test.ts`、`validateUpdaterSigningSecret.test.ts`）。
