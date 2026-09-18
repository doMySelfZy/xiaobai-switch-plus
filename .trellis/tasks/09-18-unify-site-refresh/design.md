# 技术设计：统一站点模型与余额刷新

## 1. 边界与目标

新增一个“刷新所有启用站点”的共享领域动作，统一手动全局刷新与应用级自动刷新。动作同时刷新模型和余额，负责并发、单站点隔离、结果汇总及状态回写。站点详情按钮继续保留单站点模型刷新；单站点余额按钮继续保留强制余额探测，但不再创建定时器。

主窗口 `AppInner` 只负责一个周期调度器：读取 `useSettingsStore` 的 `floatingWindow.autoRefreshMinutes`，设置变更后重新建立 interval。调度器调用共享刷新动作，不依赖当前页面。`SitesPage` 删除当前选中站点 30 秒余额轮询；`FloatingWindow` 删除自己的 interval，仅在挂载时读缓存并监听统一刷新完成事件/定期结果通知，手动刷新按钮也调用统一刷新入口。

## 2. 数据流与契约

```text
AppInner interval / SitesPage global button / FloatingWindow manual button
                         │
                         ▼
              useSiteStore.refreshAllSites()
                         │
             snapshot enabled sites + active key ids
                         │
        ┌────────────────┴────────────────┐
        ▼                                 ▼
 fetchModels(siteId, apiKeyId snapshot)   probeQuota(siteId, {force:true})
        │                                 │
        ▼                                 ▼
 modelsBySite + site fetch metadata       quotaBySite + backend quota cache
                         │
                         ▼
       aggregate per-site results + emit refresh event
                         │
                         ▼
          floating webview reads get_all_sites_quota
```

### Store contract

在 `siteStore` 增加统一动作及刷新状态，建议形状如下：

- `refreshAllSites(opts?: { notify?: boolean }): Promise<RefreshAllSitesResult>`
- `refreshingAll: boolean`
- `refreshAllSites` 在动作开始时从当前 store 快照筛选启用站点，并为模型请求传递该站点当时的 `activeApiKeyId`；单站点内部同时启动模型与余额请求，使用 `Promise.allSettled` 或等价结果隔离。
- `fetchModels` 增加内部可选 key 快照参数，公共详情按钮仍只传 site id；成功回写前再次校验当前 active key 与请求 key 一致，避免切换密钥后旧请求覆盖新状态。
- 余额刷新使用 `probeQuota(siteId, { force: true })`，保留已有 in-flight/cache-key 保护；如统一动作与其他请求撞车，沿用当前去重 promise。
- 结果按站点记录 `models: success/error` 与 `quota: success/error`，汇总 `successCount`、`failureCount` 和是否全部成功，供页面提示与测试断言。

为避免统一动作触发两次相同网络请求，`SitesPage` 全局按钮、`AppInner` 定时器、`FloatingWindow` 手动刷新都只调用 store 动作；悬浮窗不再直接调用 `refresh_sites_quota`。余额探测仍由 `probeQuota` 调用现有 `probe_site_quota`，在主窗口 store 中落地结果；为了跨窗口展示，需要一个轻量后端批量余额刷新/缓存同步命令，或让统一动作调用现有 `refresh_sites_quota` 并把其结果映射到 store。优先复用现有后端批量命令，避免重复实现余额探测协议。

## 3. 后端共享刷新

现有 `refresh_sites_quota` 已经并发探测全部启用站点并写入 `QUOTA_CACHE`。统一刷新命令应复用该函数的余额部分，并新增模型批量刷新所需的后端辅助函数，而不是在 Rust 中复制模型 HTTP 协议：

- 读取一次启用站点列表和设置，按站点异步调用现有 `fetch_site_models` 的等价内部逻辑。
- 每个站点按 active key 读取并解密；请求完成后只有 key 仍为 active 时才写入模型表/元数据。
- 模型空响应保留现有列表回退语义；单站点错误记录脱敏后的错误并继续其他站点。
- 余额继续通过 `probe_quota_for` 和 `QUOTA_CACHE` 写入。
- 命令返回每站点刷新摘要（站点 id、模型成功/失败、余额成功/失败、模型数量及错误码/脱敏消息），并在结束时发出 `sites-refresh-finished` 事件。事件只作为跨窗口失效通知，悬浮窗收到后调用 `get_all_sites_quota` 读取缓存，不携带密钥或原始敏感值。

如果通过后端统一命令实现会造成主窗口 store 无法即时得到模型明细，则保留 store 的模型并发动作作为唯一模型写回层，统一余额只调用 `refresh_sites_quota`；无论采用哪种落地，用户入口必须只有一个 `refreshAllSites`，不能在页面各自并行启动两轮任务。

## 4. 全局刷新 bug 修复策略

重点修复当前全局调用与详情按钮之间的隐式差异：

1. 批量开始时从 `useSiteStore.getState().sites` 取得最新站点，而不是闭包中的旧 `sites` 数组。
2. 为每个站点捕获 `activeApiKeyId(site)`，传入 `fetch_site_models`；不要让异步请求开始后再从可变 store 读取 key。
3. 让详情按钮和批量动作共用同一个内部 `fetchModelsForSite(siteId, apiKeyId?)`，包括空结果 `list_site_models` 回退、当前 key 校验、模型元数据更新和错误状态处理。
4. 对每站点模型/余额使用 `allSettled`，不要用一个 reject 让 `Promise.all` 跳过汇总；刷新按钮 finally 恢复状态。
5. 全局刷新完成后从 store 结果统计成功数；不使用当前实现中参数名不一致导致提示统计失真的逻辑。

## 5. 事件、设置与生命周期

- `AppInner` 订阅设置 store 的 `loaded` 与 `floatingWindow.autoRefreshMinutes`；设置更新仍由现有 `floating-settings-changed` 机制驱动，或直接通过 store 更新触发 interval 重建。
- 自动刷新 interval 只在主窗口存在一份，组件卸载时清理；页面切换、KeepAlive、托盘隐藏不影响它。
- `FloatingWindow` 保留首次缓存加载、手动刷新按钮和设置变化监听，但删除独立 interval。事件监听必须在组件卸载时解绑，避免窗口重开后重复订阅。
- 统一刷新运行期间要有 store 级 in-flight 锁；手动按钮、定时器与全局按钮撞车时返回同一个 promise，不产生第二轮请求。

## 6. 错误处理与兼容性

- API key、余额 token、请求头等敏感值不进入刷新结果事件、日志或前端提示；错误沿用现有脱敏与 `isAppError` 映射。
- 单站点模型失败时保留/清空模型列表的行为沿用现有 `fetchModels` 约定；余额失败保留上次缓存并标记错误。
- 禁用站点永远不加入统一刷新快照；刷新过程中被禁用的站点，完成回写前检查当前站点状态/key。
- 保持 `floatingWindow.autoRefreshMinutes` 字段、默认值、分钟单位、`refresh_sites_quota` 与 `get_all_sites_quota` 命令兼容，除非统一命令需要增加而不删除字段。

## 7. 测试策略

前端：

- `siteStore.test.ts`：多站点模型+余额并发、单站点失败隔离、active key 快照、in-flight 去重、禁用站点过滤、统一结果统计。
- `SitesPage.test.tsx`：全局按钮同时触发模型/余额、成功/部分失败提示、全局模型结果实际落入列表；复现并锁定“全局失败、详情成功”场景。
- `App.test.tsx` 或新增刷新调度测试：只有一个 interval、按设置间隔触发、设置变化重建、页面切换不创建新 interval。
- `FloatingWindow.test.tsx`：不再创建独立 interval；统一刷新事件后重新读余额缓存；手动按钮调用统一动作。

后端：

- 统一命令的单站点失败隔离、启用过滤、模型持久化和余额缓存更新测试。
- 原有 models/quota 命令测试继续通过。

验证命令：`pnpm test:run`、`pnpm typecheck`、`cargo test`（在 `src-tauri`）。

## 8. ModelScope 额度边界

本次统一刷新只负责现有模型与余额链路，不把 ModelScope API-Inference 的免费剩余调用次数当作可查询余额。研究确认 ModelScope 官方 OpenAPI 的 `GET /openapi/v1/magicubes/balance` 可用 Bearer Access Token 返回魔粒账户余额，但该余额与 API-Inference 免费模型调用额度不是同一口径；官方也未承诺稳定的免费次数/token 配额查询 endpoint。未来若产品明确要展示魔粒余额，应新增 ModelScope host 识别、专用来源和响应解析，并隔离于通用 billing/Sub2API 探测；本任务不猜测或依赖限流响应头。

## 9. 回滚点

优先保持改动集中在 `siteStore.ts`、`App.tsx`、`SitesPage.tsx`、`FloatingWindow.tsx`、刷新相关测试与必要的 Rust command/domain 文件。若统一后端命令出现跨窗口事件兼容问题，可先保留现有 `refresh_sites_quota` 命令和缓存协议，只回滚事件载荷与新批量命令实现；不回滚模型 active-key 快照和 `refreshAllSites` 的单入口约束。
