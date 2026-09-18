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
- 模型请求按 `(siteId, apiKeyId, baseUrl, quotaRevision)` 做 in-flight 去重；结果写回前必须验证请求版本、当前 Base URL 和 active key 仍一致。
- 批量模型请求要限制并发，并用按站点结果隔离的 settled 结果汇总；余额批量命令的 rejection 必须在创建 promise 时转为 settled 结果，不能等模型请求结束后才接住。
- Rust 余额缓存完成后通过 `sites-refresh-finished` 事件通知悬浮窗；悬浮窗只重新读取 `get_all_sites_quota`，不依赖主窗口内存状态。悬浮窗手动刷新通过 `sites-refresh-requested` 请求主窗口统一入口。
- `floatingWindow.autoRefreshMinutes` 仍是唯一刷新间隔；页面切换、KeepAlive、托盘隐藏和悬浮窗挂载都不得创建第二个 interval。

错误状态不应只看余额 `status`：如果保留旧余额但 `quota.error` 非空，该站点本轮余额刷新仍算失败。

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
