# 实施计划：统一站点模型与余额刷新

## Phase 1 — 刷新契约与状态模型

1. 在 `src/types/domain.ts` 增加统一刷新结果的类型；字段只包含站点 id、模型数量、模型/余额成功状态与脱敏错误信息。
2. 在 `src/stores/siteStore.ts` 增加 `refreshAllSites`、统一刷新 in-flight promise 和聚合状态；先快照启用站点及 active key，再并发执行每站点模型与余额刷新。
3. 把 `fetchModels` 的核心逻辑提取为可接收 API key 快照的内部函数；详情按钮和统一入口共用该函数。保留空模型回退、当前 key 校验和现有错误语义。
4. 明确余额刷新结果如何回写 store：复用 `probeQuota` 的缓存/去重；若批量命令返回汇总，则为主窗口映射到 `quotaBySite` / `quotaAttemptBySite`。

验证：先运行 `pnpm test:run -- src/stores/siteStore.test.ts` 与 `pnpm typecheck`。

## Phase 2 — 应用级定时调度与页面入口

1. 在 `src/App.tsx` 的 `AppInner` 增加唯一 interval，依赖 `autoRefreshMinutes`，等待 settings 加载后启动；组件清理时清除。
2. 调度器调用 `useSiteStore.getState().refreshAllSites()`，避免闭包中的旧站点列表和旧 key。
3. 删除 `SitesPage.tsx` 当前选中站点的 30 秒余额轮询；全局刷新按钮改为调用 `refreshAllSites`，使用统一结果做成功/部分失败/全部失败提示。
4. 保留站点详情单站点模型刷新和余额按钮，但禁止它们改变统一定时器；修复全局提示变量名/计数映射。
5. 删除 `FloatingWindow.tsx` 的独立 interval；手动刷新改为调用统一入口，统一刷新事件后重新读取 `get_all_sites_quota`。
6. 若主窗口与浮窗需要跨窗口通知，在 Rust 侧发出不含敏感值的 `sites-refresh-finished` 事件，并在浮窗解绑监听。

验证：补充并运行 `SitesPage.test.tsx`、`FloatingWindow.test.tsx` 与应用调度测试。

## Phase 3 — 后端共享刷新与事件

1. 在现有 floating/quota command 附近抽取可复用的启用站点余额刷新函数，保留 `refresh_sites_quota` 的向后兼容行为。
2. 若前端 store 直接逐站点探测无法让悬浮窗共享同一轮结果，新增统一批量 command 或扩展现有 command 返回刷新摘要；模型刷新复用 `models_fetch` 和 repo 逻辑，不复制协议实现。
3. 确保每站点 active key 在异步请求前捕获，结果写回前校验 key 仍为当前 active；错误脱敏并单站点隔离。
4. 发出统一刷新完成事件；浮窗只重新读取余额缓存，不接收 API key、token 或完整模型请求错误。
5. 增加 Rust 单元测试覆盖启用过滤、单站点失败隔离、模型落库与余额缓存更新。

验证：在 `src-tauri` 执行 `cargo test`。

## Phase 4 — ModelScope 额度边界复核

1. 保持 ModelScope 免费 API 剩余调用次数不进入本次实现；统一刷新只复用现有余额探测契约。
2. 若测试夹具需要覆盖 ModelScope 站点，只验证模型列表获取和余额 `unsupported` 的安全表现，不伪造免费额度成功结果。
3. 记录官方魔粒余额 endpoint 作为后续独立需求的研究结论，不在本任务中修改站点字段或额度协议。

## Phase 5 — 回归与性能检查

1. 验证全局刷新与详情刷新使用同一 active key 与模型结果，覆盖原始 bug。
2. 验证同一时间点击全局按钮、定时器和浮窗刷新只产生一轮请求；刷新完成后 loading 状态一定解除。
3. 验证页面切换、KeepAlive、托盘隐藏与浮窗独立显示不会增加 interval 或重复网络请求。
4. 执行完整 `pnpm test:run`、`pnpm typecheck`、`cargo test`。
5. 检查 i18n 中英文键、错误脱敏、现有 `get_all_sites_quota` / `refresh_sites_quota` 兼容性。

## 风险与回滚点

- 风险：主窗口 store 与浮窗 webview 内存隔离。回滚时保留后端 `QUOTA_CACHE` 与 `get_all_sites_quota`，仅回退事件桥接。
- 风险：统一任务与已有 TTL/in-flight 交错。回滚前保留 `probeQuota` 去重，并为统一入口使用明确 force 语义。
- 风险：批量并发放大模型请求。实现阶段应沿用现有模型探测超时，并根据测试结果限制并发；不得恢复页面级重复定时器。
- 风险：站点/API key 在请求期间变化。必须保留快照和结果写回校验。

## Verification Record

- `pnpm typecheck`：通过。
- 刷新相关回归：`src/stores/siteStore.test.ts`、`src/pages/SitesPage.test.tsx`、`src/pages/FloatingWindow.test.tsx`，共 69 tests passed。
- `cargo test --manifest-path src-tauri/Cargo.toml`：554 passed、0 failed、2 ignored。
- `pnpm test:run`：409 passed；仅仓库既有的 `generateUpdaterManifest.test.ts` 与 `validateUpdaterSigningSecret.test.ts` 因 shebang 解析问题失败，和本次改动无关。
- `git diff --check`：通过。

## Completion Checklist

- [x] `prd.md` 完成 convergence pass，Open Questions 为空。
- [x] `design.md` 与 `implement.md` 已审阅，ModelScope 免费额度查询边界已记录。
- [x] 统一刷新入口、应用级定时器、跨窗口事件和回归测试已实现。
- [x] 前端类型检查、刷新相关测试与 Rust 测试已完成。
