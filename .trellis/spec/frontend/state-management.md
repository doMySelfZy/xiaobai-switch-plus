# State Management

> zustand store 与后端调用的边界。

---

## 基本约定

- 一个领域一个 store：`src/stores/<域>Store.ts`，并从 `stores/index.ts` 统一导出。
- store 里封装 `invoke` 调用；**组件通常不直接 invoke**。
  例外：一次性的、与全局状态无关的查询（如页面打开时取一次路径列表）可以直接调。
- 组件用选择器订阅，避免整个 store 引起重渲染：

```tsx
const servers = useMcpStore((s) => s.servers);
const loadServers = useMcpStore((s) => s.loadServers);
```

- 测试里重置 store 用 `useXxxStore.setState({ ... })`（见 `McpPage.test.tsx`）。

## store 里做「写后回读」

写完数据后重新拉一次列表，保证 UI 与后端一致，而不是在前端手改数组：

```tsx
saveServer: async (input) => {
  const result = await invoke<McpSaveResult>("save_mcp_server", { input });
  const servers = await invoke<McpServerSummary[]>("list_mcp_servers");
  set({ servers });
  return result;
},
```

## 组件内部状态

纯 UI 状态（弹窗开关、当前编辑项、搜索词）用 `useState`，不要塞进 store。

## 跨窗口

**不同 webview 之间不共享 store**。悬浮窗是独立窗口，主窗口的 zustand 状态它读不到。
需要共享时：后端持久化 + Tauri 事件通知（见 `hook-guidelines.md`）。

### 统一站点刷新契约

站点模型与余额刷新由主窗口 `AppInner` 持有一个定时器，入口统一为
`useSiteStore.getState().refreshAllSites()`。站点页和悬浮窗不能各自创建周期任务：

- `refreshAllSites()` 只筛选 `site.enabled === true` 的站点，刷新开始时捕获每站点 `activeApiKeyId`。
- **一个站点的一轮刷新 = 模型 + 余额两件事**，两件事在同一个按站点 worker 里等齐。列表行的刷新指示器（`refreshingSiteIds`）跟着这一轮摘除：只等模型会让所有行先停止转圈、头部总按钮却还在转（曾经就是这样）。
- 模型请求按 `(siteId, apiKeyId, baseUrl, quotaRevision)` 做 in-flight 去重；结果写回前必须验证请求版本、当前 Base URL 和 active key 仍一致。
- 余额那一腿用逐站的 `refresh_site_quota`，**不是** `probe_site_quota`：前者探测完还写后端的悬浮窗余额缓存，后者只回给调用方——换成后者会让悬浮窗静默读到旧值。批量命令 `refresh_sites_quota` 已删除。
- 按站点并发受限（4），单个站点的模型 / 余额任一失败都要转成该站点的 settled 结果并照常摘除指示器，不影响其它站点。
- `refreshingSiteIds` 并发语义：全局启动用并集（现有 ∪ 本轮 `runIds`），全局 `finally` 只清本轮 `runIds`；单刷用累加 + `finally` 摘除。整体覆盖会洗掉并发单刷 id，导致提前灭灯而请求仍在跑。
- 余额分两份缓存：`quotaBySite` **只存** `status === "available"`（最近一次成功，金额口径），`quotaAttemptBySite` 存最近一次尝试（含失败）。列表行第二行按这个优先级取数：有成功金额显示金额，否则用尝试的 status 显示原因（`unsupported / unauthorized / invalid_data / error`），不留空白。全局刷新那一腿遇到命令 reject 时也要经 `errorQuotaAttempt()` 合成一条 error 尝试——只写成功结果会让连不上的站点永远空白。
- 额度缺失原因的文案分两套，**不得合并**：列表行（232px、会被截断）用四字摘要 `sites.quotaLabel*`，详情面板用完整句子 `sites.quotaUnsupported` 等并保留超时 / 上游 5xx 细分。同一屏出现两份完整句子就是回归。
- Rust 余额缓存完成后通过 `sites-refresh-finished` 事件通知悬浮窗；悬浮窗只重新读取 `get_all_sites_quota`，不依赖主窗口内存状态。悬浮窗手动刷新通过 `sites-refresh-requested` 请求主窗口统一入口。
- `floatingWindow.autoRefreshMinutes` 仍是唯一刷新间隔；页面切换、KeepAlive、托盘隐藏和悬浮窗挂载都不得创建第二个 interval。

错误状态不应只看余额 `status`：如果保留旧余额但 `quota.error` 非空，该站点本轮余额刷新仍算失败。

### 服务商识别与额度单位单一来源

「这个站点属于哪个服务商」只有一个判据：**Base URL 的 host**。它不写进站点记录，也不建列。

- 前端判定只在 `src/lib/siteProviderKinds.ts`；模板目录只在 `src/lib/sitePresets.ts`（纯数据）。
  `browserMock.ts` 必须 `import` 前者，**不得**再抄一份 `isXxxBase`。
- 后端对应 `quota_probe::is_opencode_go_base` / `is_modelscope_base`，两边同一口径：只接受
  `https`、host **严格相等**。加服务商时两边都要补反例测试（`api.opencode.ai`、
  `xxx.evil.com`、`http://`、`/zen/goose` 这类形似值）。
- 免除 newapi 凭据的显示条件走 `quotaCredentialHint(当前第一项 Base URL)`，只看输入、不看用户点过哪个模板。
  命中免除时**保存要省略 `newapiAccessToken` / `newapiUserId`**（`None` = 保留、`Some("")` = 清空），
  否则切一次 host 就把既有凭据洗掉。
- 空 key 的专用渠道仍要发探测：命令层放行条件用 `quota_probe::allows_empty_key_probe`（不是只看
  opencode 那一个 host），否则会落到 `empty_key_result()`，用户看不到可诊断的 401。
- 额度金额的出口只有 `formatQuotaAmountLocalized`（列表行 / 详情行 / 悬浮窗）。任何一处自己拼
  `` `$${n.toFixed(2)}` `` 都会把点数（`RAW_QUOTA`）、魔粒（`MAGICUBE`）、人民币（`CNY`）读成美元。
  新 `unit` 值必须同时进 `normalizeQuotaUnit` 与 `formatQuotaAmountParts`，否则走未知单位兜底、
  把上游原串（`MAGICUBE`）直接印进文案。

## 与后端一致性的陷阱

前端字段名必须与 Rust 的 serde 命名**逐字对齐**（Rust 端统一
`#[serde(rename_all = "camelCase")]`）。写错名字不会报错——serde 忽略未知字段，
表现为「设置改了但没生效」。

真实案例：前端 `autoRefreshSeconds` vs 后端 `auto_refresh_minutes`，刷新间隔设置
静默失效了很久。**改这类字段时两边一起改，并跑一次集成验证**。

## 浏览器开发模式

`src/lib/browserMock.ts` 提供非 Tauri 环境的 mock：每个命令一个 `case`。
新增命令时同步补 mock，否则浏览器里会报「Unknown command in browser mock」。
mock 的行为要与真实 command 的契约一致（校验、返回形状），否则开发时看不出问题、
真机上才炸。
