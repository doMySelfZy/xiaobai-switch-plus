# 魔搭魔粒余额措辞与额度来源可见性

## Background

用户 2026-09-19 反馈：「魔搭这个魔力是不是不准，我今天没怎么用，为什么剩余魔力是 0，应该满配才对」。排查过程中发现界面上那个数**无法自证来源**，两轮争议都源于此。

排查得到的事实（均为实测，非推断）：

- **上游回的是 250，不是 0**：用本机库内那把 `ms-9…6bd8`（长度 39，与 `site_api_keys.key_prefix` 一致）直连，`GET https://modelscope.cn/openapi/v1/magicubes/balance` → HTTP 200 / 226ms，`{"success":true,"request_id":"604bf8fb-86e4-4f1e-8582-04fd5ee4f759","data":{"total_balance":250,"available_balance":250,"frozen_amount":0}}`。
- **官方契约核对**（`https://modelscope.cn/.well-known/openapi.json`，版本 `1.1.0+master.20260916T070635Z`）：servers 基址 `https://modelscope.cn/openapi/v1`，全库唯一余额端点 `GET /magicubes/balance`；`available_balance` 官方描述「可用额度」、`total_balance`「总额度（可用额度 + 预扣额度）」、`frozen_amount`「预扣额度（进行中任务未返回结果时预扣减）」。`quota_probe` 的端点常量与字段读取与此一致 → **探测链没有读错字段**。
- **通用链已排除**：`/v1/usage`、`/v1/billing/usage`、`/api/user/self` 在 `api-inference.modelscope.cn` 上全部 404（未带令牌实测）。没有魔搭支路的旧构建对这个 host 只会落到「无额度接口」，编不出一个 0。
- **版本事实**：用户机器上运行的「0.1.5」其 exe 与 `src-tauri/target/release/XiaoBaiSwitchPlus.exe` 字节数（27443200）与 mtime（2026-09-19T14:16:12Z）完全一致，是**本分支当晚的构建**，不是 tag `v0.1.5`；二进制内可搜到 `magicubes/balance` 与 `api-inference.modelscope.cn` 两个字面量。
- 魔粒**不按天回满**：它是账户余额，不是每日调用计数器（`09-18-site-add-presets/prd.md:9` 已确认官方无「模型 → 魔粒」换算、无剩余次数端点）。用户「今天没怎么用 → 应该满配」的前提本身不成立。

## 现状缺陷

1. **措辞越权**：`SiteQuotaRow.tsx:229` 与 `SiteListItem.tsx:135/178/189` 对所有余额来源统一使用 `sites.quotaRemaining`（zh「剩余 {{amount}}」），魔搭因此渲染成「剩余 250 魔粒」。`09-18-site-add-presets/design.md:69` 明令魔粒不得使用「剩余可用」「额度」这类会被读成次数的措辞 —— 共用的余额行漏掉了它，用户正是这样误读的。
2. **来源不可见**：`SiteQuota.source` 与 `.endpoint` 后端已写入（`quota_probe/mod.rs:927-928`），但前端**从不渲染**（`src/` 下 `quota.source` / `quota.endpoint` 只出现在类型声明与浏览器 mock 里）。用户和开发者都无法判断某个数出自哪个端点，这是本次争议被放大的直接原因。
3. **`frozen_amount` 被丢弃**（`mod.rs:1734` 有意不显示）：当可用为 0 而存在预扣时，「0」会被读成「已花光」，而官方语义是「进行中任务预扣减」。

## Requirements

- **R1.** 魔粒口径的余额不再出现「剩余」二字：详情面板与列表行按 `unit` 分支选措辞，魔搭走「魔粒余额 N」这类中性措辞。**新增独立 i18n key**，不改写 `sites.quotaRemaining` 本身（其他来源继续用它）。
- **R2.** 详情面板把额度来源可见化：渲染 `quota.source` + `quota.endpoint`（小字或 Tooltip 均可），中英双语，**不新增任何网络请求**。
- **R3.** 魔粒为 0 时不得被读成「已花光」：要么给出 `total_balance` / `frozen_amount` 的可见口径，要么在 0 值上附加限定说明。具体形态在 design 阶段定，验收口径是「一个 0 不会让人误判账户状态」。
- **R4.** 保持既有红线：探测失败或数据缺失**绝不兜底成 0**（`mod.rs:1705-1706`）；列表与详情只用 `keyPrefix` 展示密钥；不新增任何把令牌写日志或写文件的路径。
- **R5.** 证据回填：把上面那次真机探测（端点、字段名、取值、`request_id`、以及「用户本机令牌直连、**未**核对应用内显示链路」这条边界）写进 `09-18-site-add-presets/implement.md` 第 15 条。

## Non-Goals

- 不改魔搭探测协议、不加新端点、不做「模型 → 魔粒」换算或剩余调用次数估算。
- 不动 `09-18-site-quota-short-label` 负责的「额度缺失原因短标签」；本任务只改**成功取到余额**时的措辞与来源展示。
- 不改额度刷新调度、并发上限、超时策略。

## Open question（影响范围，不阻塞 R1–R3）

当前构建里 ModelScope 站点刷新后显示什么？

- 显示 **250** → 用户看到的 0 属旧构建或旧时刻残留（余额只在内存，DB 无 quota 表、无历史，无法复盘），本任务按 R1–R3 收口；
- 显示 **0** → 应用侧另有一条 bug，第一嫌疑是 `http_client::build_client` 走的代理路径与直连不同，届时本任务追加一条复现修复项。

## Acceptance Criteria

- [ ] 魔搭站点详情面板与列表行显示「N 魔粒」，**不含**「剩余」二字。
- [ ] 其他来源（new-api 金额、`RAW_QUOTA` 点数、OpenCode Go 用量窗口）文案逐字节不变，各有测试钉住。
- [ ] 详情面板能看出这个数出自哪个 `source` / `endpoint`，且未新增请求。
- [ ] 魔粒为 0 的场景有明确措辞或补充口径，测试覆盖 0 值渲染。
- [ ] zh-CN / en-US 双语齐全，组件内无硬编码中文。
- [ ] `pnpm typecheck`、`pnpm test:run`、`src-tauri` 内 `cargo test` 通过（已知无关失败：`generateUpdaterManifest.test.ts`、`validateUpdaterSigningSecret.test.ts`）。
- [ ] `09-18-site-add-presets/implement.md` 第 15 条已回填真机证据并标注验证边界。
