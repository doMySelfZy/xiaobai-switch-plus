# 执行计划：性能优化

## 执行原则

- 按批次推进，**每批结束跑一次验证**（Rust/前端测试 + typecheck），不要在最后一次性验证。
- 每批独立提交（提交信息写清"改前为什么慢"）；不要用 `git add -A`（工作树里可能有并行会话的
  未跟踪文件，例如 `.zcode/plans/`）。
- 实施前先读代码确认根因仍成立（AI 复核结论约 35% 误报，见 `.trellis/spec/guides/index.md`）。
- 前端改动遵守 `.trellis/spec/frontend/hook-guidelines.md`：副作用可清理、依赖函数 useCallback、
  纯函数放组件外、拖拽不在 mousemove 里做重活。
- 用户可见文案全部走 i18n（新增键必须 zh-CN + en-US 都加）。

## 批次与顺序

### 批次 A：交互手感（风险低、体感收益最大）
1. A1 悬浮窗拖动 → `startDragging()`（**唯一需要真机验证的交互改动**）
2. A2 标题栏去掉 200ms 死区
3. A3 悬浮窗窗口引用稳定
4. A5 GoApplyButton 定时器门控
5. A6 ModelPicker deferred 过滤
6. A4 去掉 backdrop-filter（最后做，因为要目视比对）

**验证：** `pnpm typecheck`、`pnpm test:run`；A1/A4 需要真机目视（拖动跟手、视觉无回退）。

### 批次 B：后台工作与轮询
1. B1 站点配额门控（含 focus 探测）
2. B2 深链轮询平台化
3. B3 模型测试结果按帧批量提交
4. B4 四个页面的细粒度订阅 + columns memo

**验证：** `pnpm typecheck`、`pnpm test:run`（B1 建议补一条断言"非活动页不发请求"的单测）。

### 批次 C：输入与设置
1. C1 设置页数字输入草稿化
2. C2 `save_settings` 剪枝条件化

**验证：** `cargo test`（C2 涉及 Rust）、`pnpm test:run`；C1 补/改对应单测。

### 批次 D：网络等待与进度
1. D1/D2 `models_fetch` 并行化 + 超时预算（Rust）+ 前端进度/取消 + 保留旧列表
2. D5 启用开关乐观更新
3. D3 agent/MCP 检查超时与并行
4. D4 技能安装候选源

**验证：** `cargo test`（`models_fetch` 已有 mock server 测试可扩展）、`pnpm test:run`。

### 批次 E：后端阻塞与锁（高风险，最后做）
1. E1 写配置类命令改异步执行
2. E3 WebDAV 超时分层与锁范围
3. E2 同步快照 hash/压缩移出锁

**验证：** `cargo test`（同步相关测试是本批次的护栏）；E2/E3 若风险不可控，按 PRD 允许保留并写明原因。

## 验证命令（照抄）

```bash
# Rust（必须前置 MSVC 路径，否则链接会用到 coreutils 的 link.exe）
cd /e/xiaobai-switch && export PATH="/c/Program Files (x86)/Microsoft Visual Studio/2022/BuildTools/VC/Tools/MSVC/14.44.35207/bin/Hostx64/x64:$HOME/.cargo/bin:$PATH" && export CARGO_TARGET_DIR="E:/xiaobai-switch/src-tauri/target" && cargo test --manifest-path src-tauri/Cargo.toml

# 前端
cd /e/xiaobai-switch && pnpm typecheck && pnpm test:run
```

## 已知基线（用于判断"有没有改坏"）

- `cargo test`：540 passed / 0 failed / 2 ignored
- `pnpm test:run`：374 passed / 0 failed；**2 个文件**（`generateUpdaterManifest.test.ts`、
  `validateUpdaterSigningSecret.test.ts`）在本机是收集期 SyntaxError，属既有问题，不是新增失败
- `pnpm typecheck`：通过

## 收尾

1. 全部批次完成后派复核子代理做一次性能与正确性复核（对照 design.md 的不变量清单）。
2. 构建 NSIS 包并安装到本机：签名私钥用绝对路径 + 空密码（见项目记忆），
   安装前先杀进程，`Start-Process -ArgumentList '/S /D=...'`。
3. 真机确认：拖动悬浮窗跟手、拖主窗口无死区、设置页输入不卡、切页不卡、测试连接有进度。
4. 记录 journal + 归档任务（归档需 `--skip-branch-validation`，本仓库直接在 main 上开发）。

## 进度记录

- [x] 批次 A — 提交 `e6c0ebe`（另外 6 个弹窗的遮罩去 blur 在 `1a2cb7e`）
- [x] 批次 B — 提交 `4183385`
- [x] 批次 C — 提交 `d2397fc`
- [x] 批次 D — 提交 `88a29f3`
- [x] 批次 E — 提交 `f143619`
- [x] 任务文档与规范 — 提交 `f03fad9`
- [x] 复核（trellis-check，独立跑通全部测试并逐条核对验收标准；红线零触碰）
- [x] 构建安装：`XiaoBaiSwitch Plus_0.1.5_x64-setup.exe` 装到 `D:\Program Files\XiaoBaiSwitch Plus`（0.1.5，含签名）
- [ ] **真机人工确认**（需要人）：悬浮窗拖动跟手（前提：设置里开启悬浮窗——本机当前未开启，枚举窗口确认该窗口根本没创建）、设置页数字输入手感、去模糊后的观感、整体流畅度
- [x] 机器可验的部分已完成：界面渲染正常（UIA 读到 80 个元素、站点与余额齐全）；**标题栏双击最大化/还原经 SendInput 实测可用**（复核者标为 Important 的那条风险不成立）

## 测试基线（本次结束后）

- `cargo test`：554 passed / 0 failed / 2 ignored（基线 542 + 本次新增 12 条）
- `pnpm test:run`：407 passed / 0 failed；2 个文件仍是本机既有的收集期 SyntaxError
- `pnpm typecheck`：0 错误

## 复核提出但未处理（留给后续）

1. `TitleBar.tsx` 拖动已按 Tauri 官方形状实现，但"双击是否被拖动吞掉"已由本次真机测试证明没问题；保留现状。
2. `FloatingWindow.tsx`：若某平台系统拖动完全不发 mouseup，抑制标志会吞掉下一次正常点击（只在非 Windows 可能触发）。
3. `src-tauri/src/tray.rs:657` 托盘触发的 apply 仍在主线程（不在本批次文件范围内），应改为 `spawn_blocking`。
4. `macos_scheme.rs` 新增的 `polling_is_only_required...` 测试是恒真断言（函数体就是 `cfg!`），护栏价值低。
5. D1 的"取消"只是前端停止等待，后端请求仍会跑完（已在 UI 文案与注释里如实说明）。
6. macOS 专属分支（深链轮询、AppleScript）本机无法验证。

## 版本号提醒

本次仍是 0.1.5（与今天早些时候装的那版同号但内容不同）。若要对外发布这些性能改动，需先 bump 到 0.1.6。
