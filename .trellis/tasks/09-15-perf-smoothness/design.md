# 技术设计：性能优化

> 每条都给出"改前为什么慢 → 改什么 → 风险"。实施时先读代码确认根因仍然成立，再动手。

## 已确认良好、不要动

实施期间不要顺手改这些（评审已确认是正确的）：

- `ProxyPage.tsx:69-83`：4 秒轮询已正确用 `activePage` 门控 —— 这是本任务里其它页面要照抄的模板。
- `hooks/useDeferredTabContent.ts`、`hooks/useDeferredReady.ts`：先画骨架再挂载，形状正确。
- `components/sites/TestModelsModal.tsx` + `lib/modelProbe.ts`：逐行进度 + `AbortController` + 有界并发，
  是"慢操作该有的样子"的模板（D 批次要照它改别处）。
- 启动路径（窗口先隐藏、加载后显示；更新检查延迟 3 秒且有 `inFlight` 去重）。
- 站点图标缓存（localStorage + 在途去重 + base64 + 512KB 上限）。
- 悬浮窗配额走批量命令 + 后端缓存（`commands/floating.rs`），不要退回逐站点。
- 全项目无布局抖动、无 JS 驱动的 60fps 动画、模态全部 `destroyOnHidden`。
- `SitesPage.tsx:180-197` 的 focus 探测：这是要改的（B1），但不要顺手删掉"回到窗口补一次"的意图。

## 批次 A：交互手感

### A1 悬浮窗拖动改为系统拖动
**改前为什么慢：** `FloatingWindow.tsx:306` 在 `mousemove` 里 `void appWindow.setPosition(...)`，
每次移动都是一次 JS→Rust→`SetWindowPos` 的往返，且目标位置由"按下原点 + 窗口相对位移"推导——
窗口一移动，下一个事件的 `clientX` 就变小，形成反馈环，窗口永远追不上光标；光标移出窗口后
`document` 的 mousemove 不再触发，拖动直接冻结。

**改什么：** 越过 4px 阈值后调用一次 `appWindow.startDragging()`，把移动交给系统；
保留 `draggedRecently` 机制抑制随后的 click（拖完不误触发收起）。
`capabilities/floating-window.json` 已含 `core:window:allow-start-dragging`，无需改权限。

**风险：** 拖动结束后仍需把最终位置落盘（现在在 `mouseup` 里写），改用系统拖动后
`mouseup` 可能拿不到新位置 —— 需在拖动开始前记录、结束后用 `outerPosition()` 读一次再保存。
**这是最容易出错的一处**，实现后必须真机拖动验证：跟手、能拖出屏幕外、拖完不收起、位置被记住。

### A2 标题栏去掉 200ms 死区
**改前为什么慢：** `TitleBar.tsx:100-110` 在 Windows 上 `setTimeout(..., 200)` 后才 `startDragging()`，
拖动起始的 200ms 内窗口不动。

**改什么：** 单击立即 `startDragging()`；双击走 `toggle_maximize_window`（等价于 Tauri 自带
`drag.js` 的 `e.detail === 1 / === 2` 分支）。同时确认 `.title-bar-drag { app-region: drag }`
是否已是原生拖动区：若是，则删除重复的 JS 机制，只保留一套（真机验证后决定留哪套并写进注释）。

### A3 悬浮窗窗口引用稳定
`FloatingWindow.tsx:123` 在渲染期 `getCurrentWindow()`，每次渲染返回新对象，导致
`applyCollapsed`/`beginDrag` 身份不稳定，`mousemove`/`mouseup` 的 effect 每次都重挂监听。
改为模块级或 `useMemo(..., [])`。

### A4 去掉白付的模糊合成
`App.tsx:232-248` 的 `modal.styles.mask.backdropFilter` 与各处 `mask={{ blur: true }}`
会在 175% 缩放下对整窗做 4px 模糊；悬浮窗在**透明窗口**上做 `blur(20/24px)`，而透明窗口背后
没有页面内容可供采样。两处都去掉，保留 rgba 底色。**必须目视确认视觉没有明显回退**。

### A5 GoApplyButton 定时器门控
`GoApplyButton.tsx:44-60` 的 3 秒定时器在页面隐藏（`KeepAlivePages` 常驻）时仍跑。
按当前页 + `document.visibilityState` 门控（可直接用 B4 引入的 `usePageVisible` 之类的共享 hook）。

### A6 模型搜索 deferred
`ModelPicker.tsx:64/106-115/342-393`：`query` 每次击键同步过滤并重渲染全部 chip。
用 `useDeferredValue` 驱动过滤，输入框仍绑原始值；chip 行 `memo`。

## 批次 B：后台工作与轮询

### B1 站点配额轮询门控（已人工核实）
`SitesPage.tsx:105-108` 的定时器 `refreshAll(true)` 用 `force` 绕过 5 分钟 TTL，闸门只有
`document.visibilityState`。窗口可见 ≠ 该页可见。
**改什么：** 引入 `activePage === "sites"` 门控（照抄 ProxyPage），后台刷新不 force，
手动刷新按钮仍 force。`SitesPage.tsx:180-197` 的 focus 探测同样加门控，但不 force。

### B2 深链轮询只在需要的平台跑（已人工核实）
`useSiteDeepLink.tsx:243` 每 400ms 调 `take_pending_deep_link`；写该文件的只有 macOS 分支。
**改什么：** 由后端暴露"是否需要轮询"的能力标志（如 `#[cfg]` 返回的常量），前端据此决定是否起定时器；
运行时深链仍由 `onOpenUrl` 事件处理，Windows 上零轮询。

### B3 模型测试结果按帧批量提交
`TestModelsModal.tsx:128-144` 每个结果一次 `setResults`，且 `finishedCount` 与分组统计每次渲染
重算导致 O(N²)。改为 ref 累积 + `requestAnimationFrame` 批量 flush，行组件 memo。

### B4 细粒度订阅
`ProxyPage.tsx:55-56`、`McpPage.tsx:166-190`、`SkillsPage.tsx:47`、`RulesPage.tsx:32` 都是整店订阅，
输入一个字符即重渲染整页（含 Table 与每次重建的 columns）。
**改什么：** 改用按字段选择器；`columns` 用 `useMemo`；搜索/URL 输入拆成持有本地 state 的子组件。

## 批次 C：输入与设置

### C1 设置页数字输入
`SettingsPage.tsx:320-325/346-352/575-580` 每次击键一次 `save_settings`，且输入值来自服务端回显，
clamp 时会打断输入。改为本地草稿 + 失焦（或 400ms 防抖）提交；不要把 clamp 结果回填到正在输入的框。
`.trellis/spec/frontend/hook-guidelines.md` 未覆盖这种"输入即提交"，实现时把做法写进该 spec。

### C2 save_settings 不再全量剪枝
`commands/settings.rs:33` 每次都 `backup::prune_all(max_backup_copies)`，遍历 5 个备份目录。
改为：仅当 `max_backup_copies` 真正变化时剪枝；其余情况跳过。

## 批次 D：网络等待与进度

### D1/D2 协议检测与模型拉取
`models_fetch/mod.rs`：`detect_protocol` 5 次串行 × 15s = 最坏 75s；`fetch_models` 最坏 45s。
**改什么：** 这 5 次其实是对**同一个 URL** 的 5 种鉴权/UA 组合，不是 5 个不同端点——
用 `join_all` 并行发出、按优先级取第一个成功，最坏降到 ~一个超时周期；
再把探测类请求的超时从 15s 降到 8s（与 `quota_probe` 一致）。
前端：按钮显示进度（第几种方式）、可取消；`fetchModels` 刷新期间保留旧列表不清空
（`siteStore.ts:204` 现在的 `modelsBySite[siteId] = []` 要改）。
**注意：** 探测失败语义要区分"连不上/超时"与"认证被拒"，只有后者才值得继续换鉴权方式。

### D3 agent/MCP 版本检查
`adapters/agent_update.rs`、`adapters/mcp_update.rs` 用 `std::process::Command` 串行跑 npm，
无超时；且每次进页面都跑。改为：子进程加超时并可 kill、并行化、结果缓存一段时间；
MCP 那个 `reqwest::Client::new()` 无超时要补上超时（否则可能永久挂起）。

### D4 技能安装候选源
`commands/skills.rs:605-624` 串行 4 个候选、各 45s。改为短超时探测首个可用候选或并行取首个成功。

### D5 启用开关乐观更新
`SitesPage.tsx:276-331` 在 `enabled === false` 时先 `await loadStatus({ force: true })`
（会串行 spawn 4 个 CLI 进程）才更新 UI，开关看起来"点了没反应"。
改为先乐观更新站点，再后台刷新状态且不 force。

## 批次 E：后端阻塞与锁（高风险）

### E1 写配置类命令移出 UI 线程
Tauri 的普通 `#[tauri::command] fn`（非 async）在 IPC 处理线程上同步执行**不经过线程池**
（已核对 `tauri-macros` 生成的 `kind.block(result, resolver)` 路径）。`apply_site`、
`apply_mcp_servers`、`switch_site_route`、`scan_existing_mcp` 等命令内部做文件写入、fsync、
备份与剪枝，会阻塞窗口消息泵。
**改什么：** 改为 `#[tauri::command(async)]`（或内部 `spawn_blocking`），保持既有 `try_lock_*`
目标级锁不变。`list_target_status` 已经是这个形状，照它改。

### E2 同步快照的 hash/压缩移出 DB 锁
`sync.rs:330-337` 在 `with_conn` 内调用 `app_backup::create_backup_in`，后者做
`VACUUM INTO` + `sha256_file` + zip。`VACUUM INTO` 需要连接，但 hash/压缩只需要那个快照文件。
**改什么：** 锁内只做 `VACUUM INTO`，锁外做 hash 与打包。
**不变量：** 快照与打包结果必须仍然一致（不能出现"打包了半写状态"）；备份包内条目名与
manifest 契约不得变。

### E3 WebDAV 超时与锁范围
`webdav.rs:37-41` 客户端 300s 超时被所有请求继承；`list_webdav_backups` 等只读操作也抢
`webdav_operation` 锁，而自动同步持锁期间包含网络传输。
**改什么：** 分层超时（连接 10s / 读写较长）、只读列表不抢操作锁（或 `try_lock` 返回忙碌）、
保存配置不等同步完成（scheduler 重启 fire-and-forget）。

## 不变量清单（改完必须自证仍然成立）

1. 兼容红线：manifest 名、远端目录默认值、备份前缀双识别、包内条目名、`xiaobai_` 命名空间。
2. 同步四不变量（`.trellis/spec/backend/webdav-sync.md`）：指纹算法版本、防静默覆盖、
   下载落地再记账、启动恢复协议。
3. 悬浮窗：拖动落盘只在结束时发生一次；点击与拖动不互相误判。
4. 深链：Windows 上运行时深链仍能打开（去掉轮询不等于去掉能力）。
5. i18n：新增文案双语齐全，插值双花括号。
