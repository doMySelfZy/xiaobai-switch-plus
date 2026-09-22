# 执行计划：删除悬浮窗功能

工作流：inline（无子代理分发，改动集中且已完全勘查定位）。

## 有序清单

### 阶段 A — 后端（一轮改完再编译）
1. `commands/quota.rs`：新增/迁入 `refresh_site_quota`（薄封装 `probe_quota_for`，无缓存写入）。
2. 删 `commands/floating.rs` 整个文件。
3. `commands/mod.rs`：删 `pub mod floating;`（3）、`pub use floating::*;`（23）。
4. `lib.rs`：删 `mod floating_window;`（14）、`init_floating_window` 调用块（131-133）、`invoke_handler` 里 10 个悬浮窗命令注册（266-276 区间，保留 `refresh_site_quota`）。
5. 删 `src-tauri/src/floating_window.rs`。
6. `domain/mod.rs`：删 `floating_window` 字段（645-647）、`FloatingWindowSettings` + `Default` + 两个 helper（650-688）、`Default for AppSettings` 初始化行（940）、`SiteQuotaSummary`（945-954）。
7. 删 `src-tauri/capabilities/floating-window.json`。
8. `src-tauri/tauri.conf.json`：删 `windows` 数组里 `label: "floating"` 对象。
9. 确认 `AppSettings` 无 `#[serde(deny_unknown_fields)]`（否则旧库残留键会反序列化失败）。
10. `cargo test`（在 `src-tauri`）。

### 阶段 B — 前端删除
11. 删 `src/pages/FloatingWindow.tsx`、`FloatingWindow.test.tsx`、`FloatingWindow.new.tsx`、`FloatingWindow.tsx.tmp`、`src/floating.tsx`、`floating.html`。
12. `vite.config.ts`：删 `floating` 多入口（35-38）。
13. `App.tsx`：`autoRefreshMinutes` selector → 常量 2 分钟；删 `sites-refresh-requested` 监听（175-190）。
14. `SettingsPage.tsx`：删 `notifyFloatingSettingsChanged`（24-30）、`groupFloatingWindow`（291-345）及调用（340）。
15. `types/domain.ts`：删 `floatingWindow?`（387-395）、`SiteQuotaSummary`（564-570）。
16. `siteStore.ts`：删 `emitSitesRefreshFinished`（54-59）与 4 处调用（648,682,731,737）；更新注释（121-122,219,725,729）；核对 `refresh_site_quota` 调用（240）错误处理不依赖后端回读。
17. `browserMock.ts`：删 `floatingWindow` 默认/拷贝/patch（79、427-429、747-749）、`get_all_sites_quota`（1549）、8 个悬浮窗命令 mock（1696-1717）；保留 `refresh_site_quota`（1689-1694）。

### 阶段 C — 测试与原型清理
18. `siteStore.test.ts:785`：调整/删除 floating 通知断言。
19. `SettingsPage.test.tsx:579,603`：删除/调整刷新间隔保存用例。
20. 注释清理（`SitesPage.tsx:165,198-199`、`SiteListItem.tsx:339`、`BackupQuickPopover.test.tsx:53`）。
21. 删 `prototypes/floating-window*.html`（约 10 个）+ `prototypes/simple-float-button.html`。

### 阶段 D — 验证（AC）
22. `pnpm typecheck`
23. `pnpm test:run`
24. `cargo test`（若阶段 A 后又动过后端）
25. 残留 grep 扫描（AC1，排除 `.trellis/`、`dist/`）。
26. `pnpm run build` 确认无 `dist/floating.html`（可选，或留待用户要求构建时验证）。

## 验证命令

```bash
# 前端
pnpm typecheck && pnpm test:run
# 后端
cd src-tauri && cargo test
# 残留扫描（在仓库根）
grep -rniE "floating|悬浮窗|QUOTA_CACHE|get_panel_direction|SiteQuotaSummary|floatingWindow|sites-refresh-(requested|finished)|get_all_sites_quota|get_floating_quota_summary" src src-tauri/src --include="*.ts" --include="*.tsx" --include="*.rs"
```

## 风险文件 / 回滚点

- `lib.rs`、`commands/mod.rs`、`domain/mod.rs`：删引用漏一处即编译失败 —— 阶段 A 一轮改完统一编译。
- `siteStore.ts`：`refresh_site_quota` 是保留命令，误删会断掉主窗口单站刷新 —— 只删 emit 链，不动该命令调用。
- `App.tsx`：改错刷新常量或 effect 依赖会导致主窗口不再定时刷新 —— 保留 effect，只换数据来源。
- 回滚：纯删除，`git revert` 本次提交即可，无数据破坏。
