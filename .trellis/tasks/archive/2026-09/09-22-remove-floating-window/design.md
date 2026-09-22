# 设计：删除悬浮窗功能

## 架构影响

悬浮窗是一条从主程序旁伸出的独立分支：独立 webview 窗口 + 独立入口 + 后端缓存 + 一对跨窗口事件。删除即把这条分支整段剪掉，主程序（站点管理、余额刷新、设置）保持原样。三个耦合点必须精确处理，其余整删。

## 三个耦合点的处理口径

### 1. `refresh_site_quota`（保留，迁移）
- 现状：住在 `commands/floating.rs`，探测后写 `QUOTA_CACHE`，失败时回读缓存并附错误。
- 目标：迁到 `commands/quota.rs`，薄封装 `probe_quota_for`：
  ```rust
  #[tauri::command]
  pub async fn refresh_site_quota(
      state: State<'_, AppState>,
      site_id: String,
  ) -> AppResult<SiteQuota> {
      crate::commands::quota::probe_quota_for(&state, &site_id).await
  }
  ```
  （若 `refresh_site_quota` 就写在 `quota.rs` 内，可直接调用同模块函数。）
- 行为变化：失败不再回读旧缓存，直接 `Err`。前端 `siteStore.ts` 自持 quota 状态，单站刷新失败时保留上次值由前端负责 —— 实现时核对 `siteStore.ts:240` 的 catch 分支确认不依赖后端回读。

### 2. `autoRefreshMinutes`（改为硬编码 2 分钟）
- `App.tsx:145-147` 删除 selector，改为模块级常量 `const AUTO_REFRESH_MS = 2 * 60_000;`。
- `App.tsx:165-173` 的 effect 依赖从 `[autoRefreshMinutes, ...]` 去掉该变量，用常量。
- 设置页刷新间隔 UI 整块删除（不保留通用设置项）。

### 3. 跨窗口事件（整删）
- `sites-refresh-requested`：无 emitter，`App.tsx:175-190` 监听是死代码，删。
- `sites-refresh-finished`：唯一监听方是悬浮窗，`emitSitesRefreshFinished` 及 4 处调用删。

## 数据 / 兼容

- 无 schema 变更。`AppSettings` 去掉 `floating_window` 字段后，旧库 `settings` JSON blob 里的 `floating_window` 键被 serde 反序列化时忽略（默认行为，非 `deny_unknown_fields`）—— 实现时确认 `AppSettings` 未加 `#[serde(deny_unknown_fields)]`，否则旧库会反序列化失败。
- `sync.rs` 的 `FINGERPRINT_TABLES` 不含 floating，无需改。
- `SiteQuotaSummary`（前后端）仅悬浮窗使用，随删。

## 编译顺序风险

- 后端删除会牵连编译：先迁 `refresh_site_quota` → 再删 `floating.rs` 里其余内容 → 再删 `mod.rs` 引用 → 再删 `lib.rs` 注册与 `mod`/init → 最后删 `domain` 结构体。顺序反了会出现"函数已删但仍被引用"的中间态编译错误，但只要一轮改完再编译即可。
- `get_panel_direction` 有 unused `app` 之外的告警历史 —— 随文件删除，无需单独处理。

## 生成文件

- `src-tauri/capabilities/floating-window.json` 是源；`gen/schemas/capabilities.json` 是生成产物。删源文件，生成文件下次 `tauri dev`/`build` 自动重生，不手改。

## 回滚

- 纯删除任务，回滚 = `git revert` 本次提交。无数据破坏（不动 DB、不动同步协议值）。

## 验证策略

- 后端：`cd src-tauri && cargo test`（编译 + 缓存单测已随文件删除，其余测试须绿）。
- 前端：`pnpm typecheck` → `pnpm test:run`。
- 残留扫描：AC1 的 grep 清单（排除 `.trellis/`、`dist/`）。
- 构建产物：`pnpm run build` 确认无 `dist/floating.html`。
