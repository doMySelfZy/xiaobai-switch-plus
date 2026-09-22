# 删除悬浮窗功能

## Goal

彻底移除悬浮窗（floating window）特性 —— 前后端代码、独立窗口、设置项、专用事件与遗留原型/临时文件全部清理干净，让代码库回到没有该特性的状态。主窗口的既有能力（站点余额定时刷新、单站刷新）不受影响。

用户价值：悬浮窗多轮改版后仍不满足预期，用户决定放弃该特性（"删除所有悬浮窗功能吧不搞了"）。移除后减少维护面与包体、消除死代码与命令名不匹配缺陷。

## Background / 确认事实（均来自代码勘查）

- 悬浮窗是独立 webview 窗口（label `floating`，URL `/floating`），入口 `floating.html` → `src/floating.tsx` → `src/pages/FloatingWindow.tsx`。
- 后端 `src-tauri/src/floating_window.rs` 管理窗口生命周期；`src-tauri/src/commands/floating.rs` 提供 11 个命令 + `QUOTA_CACHE`（`Lazy<Mutex<HashMap>>`）后端缓存。
- **命令名不匹配缺陷**：前端 `FloatingWindow.tsx:170,183` 调用 `get_floating_quota_summary`，后端并无此命令，只注册 `get_all_sites_quota`（`lib.rs:266`）—— 悬浮窗面板一直空白的根因。两者都在删除范围内。
- **`refresh_site_quota` 不是悬浮窗独有**：`siteStore.ts:240` / `SitesPage.tsx:198` 用它做主窗口单站余额刷新。它当前住在 `commands/floating.rs:73-94`，删文件前必须迁移并去掉 `QUOTA_CACHE` 写入后保留。
- **`autoRefreshMinutes` 驱动主窗口轮询**：`App.tsx:145-147,165-173` 读 `settings.floatingWindow.autoRefreshMinutes ?? 5` 作为主窗口 `refreshAllSites()` 的定时刷新间隔。字段名义属悬浮窗，功能属主窗口。
- **`sites-refresh-requested` 已是死代码**：全仓库无任何 emitter，只有 `App.tsx:180` 在 listen；删除安全。
- **`sites-refresh-finished` 唯一监听方是悬浮窗**：emit 链在 `siteStore.ts:54-59,648,682,731,737`（`emitSitesRefreshFinished`）；悬浮窗删除后无监听方，emit 链可删。
- 数据库无独立 floating 列/表：悬浮窗设置只是 `AppSettings` 序列化 JSON 里的 `floating_window` 键。删除 Rust 结构体字段后旧库残留键被 serde 忽略，**无需迁移、无需改 schema、无需动 `sync.rs` 的 `FINGERPRINT_TABLES`**。

## Requirements

R1. 删除后端悬浮窗模块与命令
- 删 `src-tauri/src/floating_window.rs` 整个模块及 `lib.rs:14` 的 `mod` 声明、`lib.rs:131-133` 的 `init_floating_window` 启动调用。
- 删 `commands/floating.rs` 里除 `refresh_site_quota` 外的全部命令、`QUOTA_CACHE`、`summarize`、`get_all_sites_quota`、`get_panel_direction`、缓存单测。
- 从 `lib.rs` 的 `invoke_handler` 移除 10 个悬浮窗命令（`get_all_sites_quota` / `toggle_floating_window` / `set_floating_window_collapsed` / `show_floating_window_cmd` / `hide_floating_window_cmd` / `save_floating_window_position` / `set_floating_window_enabled` / `set_floating_window_refresh_interval` / `reset_floating_window_position` / `get_panel_direction`），保留 `refresh_site_quota`。

R2. 迁移并保留 `refresh_site_quota`
- 迁到 `commands/quota.rs`，实现为对 `probe_quota_for(&state, &site_id).await` 的薄封装：成功返回 `SiteQuota`，失败传播 `Err`（去掉"回读缓存"逻辑 —— 前端 store 自持最近值）。
- `commands/mod.rs` 去掉 `pub mod floating;` 与 `pub use floating::*;`（迁移后不再需要）。
- 前端 `siteStore.ts:240` 对 `refresh_site_quota` 的调用与错误处理需在实现时核对，保证单站刷新仍可用。

R3. 删除后端设置结构
- 删 `domain/mod.rs` 的 `AppSettings.floating_window` 字段（645-647）、`FloatingWindowSettings` 结构体 + `Default` + `default_floating_refresh_interval()` + `clamp_floating_refresh_interval()`（650-688）、`Default for AppSettings` 里的初始化行（940）、`SiteQuotaSummary` 结构体（945-954）。

R4. 删除前端悬浮窗代码与入口
- 删 `src/pages/FloatingWindow.tsx`、`src/pages/FloatingWindow.test.tsx`、`src/floating.tsx`、`floating.html`。
- 删遗留临时文件 `src/pages/FloatingWindow.new.tsx`、`src/pages/FloatingWindow.tsx.tmp`（勘查确认无 `.old.tsx`）。
- `vite.config.ts:35-38` 移除 `floating` 多入口，保留 `main`。
- `src-tauri/tauri.conf.json` 移除 `windows` 数组里 `label: "floating"` 的窗口对象。
- 删 `src-tauri/capabilities/floating-window.json`（`gen/schemas/capabilities.json` 为生成文件，下次构建自动重生，不手改）。

R5. 主窗口刷新间隔硬编码 2 分钟（用户决策）
- `App.tsx` 把 `autoRefreshMinutes` 来源改为硬编码常量 2 分钟，删除对 `floatingWindow.autoRefreshMinutes` 的读取。
- 删除设置页里的刷新间隔配置 UI 与相关字段（不迁移为通用设置项）。

R6. 删除前端设置/类型/mock/事件
- `SettingsPage.tsx`：删 `notifyFloatingSettingsChanged()`（24-30）、整个 `groupFloatingWindow` 分组（291-345）及其调用（340）。
- `types/domain.ts`：删 `AppSettings.floatingWindow?`（387-395）、`SiteQuotaSummary`（564-570）。
- `browserMock.ts`：删 `floatingWindow` 默认值/拷贝/patch（79、427-429、747-749）、`get_all_sites_quota` 分支（1549）、8 个悬浮窗命令 mock 分支（1696-1717）；`refresh_site_quota` mock 分支（1689-1694）保留。
- `App.tsx`：删 `sites-refresh-requested` 监听（175-190，死代码）。
- `siteStore.ts`：删 `emitSitesRefreshFinished`（54-59）及 4 处调用（648,682,731,737）；更新提及悬浮窗的注释（121-122,219,725,729）。

R7. 删除原型文件
- 删 `prototypes/` 下所有 `floating-window*.html` 与 `simple-float-button.html`（共约 11 个）。

R8. 更新受影响测试
- `siteStore.test.ts:785` "notifies the floating window" 用例：随 emit 删除调整断言。
- `SettingsPage.test.tsx:579,603` 校验刷新间隔保存的用例：随设置 UI 删除移除或调整。
- 低优先注释清理（`SitesPage.tsx`、`SiteListItem.tsx`、`BackupQuickPopover.test.tsx`）随手更新，不影响功能。

## Acceptance Criteria

- AC1：全仓库 `grep -ri "floating|悬浮窗|QUOTA_CACHE|get_panel_direction|SiteQuotaSummary|floatingWindow|sites-refresh-requested|sites-refresh-finished|get_all_sites_quota|get_floating_quota_summary"`（排除 `.trellis/`、`dist/`）无残留代码引用。
- AC2：`cargo test`（在 `src-tauri`）通过，无编译错误、无未使用告警。
- AC3：`pnpm typecheck` 与 `pnpm test:run` 通过。
- AC4：应用启动不再创建 floating 窗口；主窗口站点余额每 2 分钟自动刷新一次仍生效；站点列表单站刷新按钮仍生效。
- AC5：`pnpm run build` 只产出主入口，不再产出 `dist/floating.html`。

## Out of Scope

- 不迁移刷新间隔为通用可配置设置项（用户已决策硬编码 2 分钟）。
- 不改数据库 schema、不改 `sync.rs` 指纹表、不做数据迁移（旧库 JSON 残留键由 serde 自动忽略）。
- 不动 `dist/` 旧构建产物的手工清理（下次 build 自动覆盖）。
- 不重新构建/本机安装（除非用户后续要求）。
