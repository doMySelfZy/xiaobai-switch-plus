# 设计：添加站点预置模板（opencode-go / 魔搭）

## 1. 边界与不变量

- **模板只是预填器，不是身份来源。** 站点属于哪个服务商，唯一判据仍是 Base URL host。模板目录不写入站点记录，也不新增列。
- **余额端点 origin 写死为常量**，不从站点 `base_url` 派生：魔搭推理 host 与 openapi host 不同域（`api-inference.modelscope.cn` vs `modelscope.cn`），派生会把「用户能控制的 host」拼进探测请求。
- `SiteQuota` 的金额字段语义不改：魔粒余额借用 `remainingUsd/totalUsd` 这两个 number 槽位，靠 `unit` 区分口径，与既有 `RAW_QUOTA` 点数口径同一套做法。
- 不显示 `frozen_amount`。它需要额外一句解释才能不被读成「已花费」，收益不抵认知成本。

## 2. 为什么不加 provider 列

`capabilities_json` 是 `Record<string, boolean>`（`src/types/domain.ts:58-59`、`src-tauri/src/capabilities.rs:29-48`），装不下字符串；真正的方案是加列。加列要覆盖 `apply_schema` 的每个「库已存在」分支去跑增量补齐（AGENTS.md 记录的失效模式：**漏分支不报错，老库静默拿不到新列**）。

而 host 白名单这条路的正确性已经被 opencode-go 验证过：`quota_probe/mod.rs:188-201` 严格相等 + path 段相邻判定，反例测试钉死 `api.opencode.ai` / `opencode.ai.evil.com` / `http://`（`:3771-3785`）。用同一形状判魔搭，额外好处是**用户手填官方 host 也能拿到同样的免除和探测**，不必非得从模板进来。故：不加列。

## 3. 前端结构

```
src/lib/siteProviderKinds.ts   （新增，单一来源）
  isOpenCodeGoBase(url)        ← 从 browserMock.ts:672-685 搬过来，browserMock 改为 import
  isModelScopeBase(url)        （新增：https + host == "api-inference.modelscope.cn"）
  sitePresetFor(url)           → 命中的模板描述符 | null
  quotaCredentialHint(url)     → 该 host 需要哪些额外凭据
src/lib/sitePresets.ts         （新增，目录常量）
  [{ id, nameKey, baseUrls, protocol, requiresNewapiCreds, quotaNoteKey }...]
  含 `custom` 一项，等价于现状空表单
```

- 目录放独立文件、纯数据，避免 `SiteFormModal` 变胖。
- 免除提示的显示条件走 `sitePresetFor(当前第一项 Base URL)`，**不看用户点过哪个模板**，满足 prd R2 的「随输入变化」。
- 预填继续经 `SiteFormInitialValues`（`SiteFormModal.tsx:101-108`）与 `SitesPage.tsx:89-102` 已有通路；该类型缺 `requiresNewapiCreds` 表达力，但免除判断已经由 host 单独得出，因此**不需要扩这个类型**。

## 4. 后端结构

`src-tauri/src/quota_probe/`：

- `is_modelscope_base(&str) -> bool`，与 `is_opencode_go_base` 并列，同一套 `Url` 解析与测试风格。
- 新 `QuotaSource::MagicubeBalance`（序列化名 `magicube_balance`），Rust `domain/mod.rs:352-363` 与 TS `types/domain.ts:540-548` 同时加。
- `probe_quota` 内新增早退分支，位置在 opencode-go 分支之后、空 key 短路之前（`quota_probe/mod.rs:1704-1724`），保持「专用来源不进通用 billing 链」的既有隔离约定（`.trellis/tasks/09-18-unify-site-refresh/design.md:100`）。
- 命令层放行条件从 `!is_opencode_go_base` 改为「两个已知 host 都放行」（`commands/quota.rs:83-85`）。否则空 key 的魔搭站点会在命令层被 `empty_key_result()` 短路，拿不到可诊断的 401 —— 这正是 opencode-go 已经踩过的形状。
- 请求：`GET https://modelscope.cn/openapi/v1/magicubes/balance`，`Authorization: Bearer <active key>`。响应按 `{success, request_id, data{...}}` 解析，`data` 及三个字段全部按可选处理。

状态映射（不猜语义，宁可判失败）：

| 上游 | 结果 |
|---|---|
| 200 且 `data.available_balance` 是有限数 | `available`，`unit = "MAGICUBE"` |
| 200 但 `data` 缺失 / 值非数值 / NaN | `invalid_data` |
| `success: false` | `invalid_data`，`error` 记 `code` 字段（不记 `message`，防回显） |
| 401 / 403 | `unauthorized` |
| 5xx / 超时 / 传输失败 | `error` |

映射表里的 `error` 文本只放状态码与 `code` 这类稳定标识，绝不含请求头、令牌或上游自由文本。

## 5. 单位与展示

`formatQuotaAmountParts`（`src/lib/quotaProbe.ts:56-88`）当前对未知 unit 走兜底分支、把 unit 原样拼进文本（`quotaProbe.ts:83-87`），因此 `"MAGICUBE"` 必须显式加分支，否则会渲染成 `1000.5 MAGICUBE`：

- `normalizeQuotaUnit` 识别 `MAGICUBE` / `魔粒`；
- 新增 `sites.quotaUnitMagicube`（zh「魔粒」/ en "Magicubes"）；
- 数值用千分位、保留上游给的小数精度；`Intl.NumberFormat("zh-CN", { maximumFractionDigits: 2 })`，不加货币符号；
- `remainingUsd = available_balance`，`totalUsd = total_balance`（仅当两者都可用才组百分比，`quotaRemainingPercent` 见 `quotaProbe.ts:115-119`），`usedUsd = null`，`unlimited = false`，`expiresAt = null`。

已知坑：`formatQuotaAmount`（非 i18n 版，`quotaProbe.ts:90-95`）对点数分支返回硬编码 `" RAW_QUOTA"` 字面量。加魔粒时**不要照抄**这条分支；本任务只走 `formatQuotaAmountLocalized`。

## 6. 权衡

- **限流**：官方规范未公布 balance 端点阈值（示例里出现过 `100 requests per minute`，非承诺）。我们的成本是每轮刷新每站点 1 次请求，默认 5 分钟间隔下远低于该量级；不做额外节流，也不为它单独开定时器（`autoRefreshMinutes` 仍是唯一间隔，见 `.trellis/spec/frontend/state-management.md`）。
- **口径风险**：魔粒与「能调多少次」无官方换算。展示层因此只用「魔粒余额」措辞，不使用「剩余可用」「额度」这类会被读成次数的词。
- **暴露面**：`modelscope.cn` 是单点常量，上游改域名即失效并落到 `error`，不会静默给出错误余额。

## 7. 分阶段

- **M1 纯前端**：`siteProviderKinds.ts` + `sitePresets.ts` + 模板入口 + newapi 凭据免除 + `browserMock` 去重。可独立发布，即使 M2 未做，魔搭站点也只是显示「无额度接口」短标签（`.trellis/tasks/09-18-site-quota-short-label`）。
- **M2 跨层**：Rust host 判定 + 探测支路 + `QuotaSource` 新值 + `unit=MAGICUBE` 前端渲染 + 悬浮窗自动继承（同走 `probe_quota_for`，无需改事件协议）。

## 8. 回滚点

- M1 出问题：把模板目录收成只有 `custom` 一项，其余代码不动。
- M2 出问题：删掉 `probe_quota` 里的魔搭早退分支即回到 `unsupported`；`QuotaSource` 新枚举值留着不影响旧数据（它是输出字段，全仓无输入读取路径，仅 `quota_source_rank` 内部排序用到，`quota_probe/mod.rs:1059-1071`）。
- 不回滚 `is_opencode_go_base` 的既有边界测试。
