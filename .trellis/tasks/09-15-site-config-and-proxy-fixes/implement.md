# 执行计划：站点配置精简、请求头全局化与代理/MCP 修复

## 前置

- 设计依据：本目录 `design.md`；兼容红线见 `prd.md` 的 Constraints。
- **并行会话**：`09-15-perf-smoothness`（in_progress）会改 `src-tauri/src/models_fetch/mod.rs` 的 `detect_protocol` 与 `src-tauri/src/sync.rs` 的锁范围。**S6 必须放在它之后**；S1–S5 与它的文件不重叠（已核对：perf 涉及 FloatingWindow/TitleBar/SettingsPage/SitesPage 定时器/useSiteDeepLink/backdrop-filter/命令线程化，本任务不碰这些）。
- 提交纪律：本仓库常有并行会话改同一棵工作树，**只按显式路径暂存**（`git add <path>`），提交前用 `git diff --cached --stat` 确认没有别人的文件。
- 构建环境（Windows / Git Bash）：

```bash
cd /e/xiaobai-switch && export PATH="/c/Program Files (x86)/Microsoft Visual Studio/2022/BuildTools/VC/Tools/MSVC/14.44.35207/bin/Hostx64/x64:$HOME/.cargo/bin:$PATH" && export CARGO_TARGET_DIR="E:/xiaobai-switch/src-tauri/target"
```

## 实施步骤

### S1 本地代理：端口切换 bug 与"运行意图 / 接管集合"解耦（R3.4 / R3.5）

改动点：
- `src-tauri/src/commands/proxy.rs:83-115`：改端口不再走会清空接管集的 `stop()`；抽出"重启监听但保留 targets"的路径（落库 → 重启 → 按 `targets` 重新写客户端配置）。
- `src-tauri/src/local_proxy/mod.rs:82-120`：让 `keep_enabled = true` 语义真正被使用——停止代理时置 `enabled=false` 并把目标写回直连，**保留 `local_proxy_targets`**；启动时按 targets 恢复接管。
- `src-tauri/src/local_proxy/server.rs:200-212`：`stop()` 与新重启路径的行为对齐。
- `src-tauri/src/commands/proxy.rs`、`lib.rs` 的退出清理路径：确认遍历的是保留后的 `targets`，退出时能正确写回直连。

验证：
- 新增单测：①停止后 `local_proxy_targets` 仍在；②改端口后 `targets` 不被清空且写回的地址使用新端口；③重启后按 targets 恢复接管。
- 手动：改端口 → 检查 DB（`settings` 的 `localProxyTargets`）与 CLI 配置文件一致 → 退出应用 → 确认 CLI 配置写回直连、不再指向本机端口。

回滚点：本步独立成一次提交；回退即可恢复旧行为。

### S2 本地代理页界面精简（R3.1 / R3.2 / R3.3）

改动点：
- `src/pages/ProxyPage.tsx:212-214`：删除标题栏的启停按钮，只保留 `:221-227` 的开关。
- `:344-349`：「刷新」按钮与 4 秒轮询合并（删除或改为轮询期间的禁用态，实现时选一并说明理由）。
- `:136-138`：端口保存失败时重新拉取状态，避免输入框在 4 秒后跳到新值。
- 文案：`portHint` 改为"修改后会自动重启监听并重写接管地址"；新增"停止代理会把所有已接管目标写回直连，重新启动会按上次选择恢复"。
- i18n 双份（`zh-CN.json` / `en-US.json`）+ `browserMock.ts` 同步。

验证：`pnpm typecheck` + `pnpm test:run`；手动过一遍启停、改端口、全部恢复直连。

### S3 请求头全局化（R2）

改动点：
- `src-tauri/src/db/migrate.rs`：新增 `local_proxy_headers` 表（`CREATE TABLE IF NOT EXISTS`），挂进 `ensure_incremental_schema`（**每个"库已存在"分支都要跑到**）。
- `src-tauri/src/sync.rs`：把 `local_proxy_headers` 加进 `FINGERPRINT_TABLES`（9 → 10），**同时**把 `FINGERPRINT_ALGORITHM_VERSION` 递增为 2，并更新 `:711-729` 的护栏测试（它会把版本号钉死在表清单上，是设计意图不要绕过）；`LEGACY_FINGERPRINT_ALGORITHM_VERSION` 保持 1。
- 新增 `repo` 层的读写（复用 `repo/site.rs:60-87` 的 `validate_proxy_headers` + `Crypto` 加密口径），配套 `commands`（注册进 `lib.rs`）。
- `src-tauri/src/local_proxy/server.rs:55-82`：`resolve()` 多读一份全局头；`forward.rs:146-178`：`merge_headers` 改三段合并（客户端 → 全局 → 站点覆盖，站点胜），同步更新其单测。
- 前端：`ProxyPage.tsx` 新增「默认请求头」卡片（复用 `ProxyHeaderEditor` + `parseProxyHeadersJson`）；`SiteFormModal.tsx:503-515` 文案改为「本站覆盖（可选）」并说明"留空则使用代理页的默认请求头"。
- i18n 双份 + `browserMock.ts`。

验证：
- 单测：全局生效、站点同名覆盖、站点未配时用全局、受保护头仍禁改、密钥仍以密文落库。
- **跨设备同步回归**：改一条全局头后本地指纹发生变化（证明会被判定为数据变更并上传）；构造 `fingerprintAlgorithm` 缺失（=1）的 manifest，确认判定为 `Incompatible` 并给出"请先升级对端"提示，而不是静默覆盖。

### S4 站点表单重排与协议自动化（R1）

改动点：
- `src/components/sites/SiteFormModal.tsx`：
  - 备注（`notes`）移到基本信息区；删除 `shouldOpenAdvanced()` 里"备注非空就展开"的分支（`:32-34`）。
  - 协议下拉 label/提示改为"默认自动检测；可手动指定"；删除"不确定时可留空"的错误文案（i18n `protocolHint`）。
  - 新建站点保存成功后，若用户未在本对话框内显式选过协议，后台调用 `test_site_connection` 并把结果写回该站点（复用 `:210,214` 的回填逻辑）。
  - NewAPI 令牌 / 用户 ID 移出站点连接配置——**首选**移到站点详情的额度区域（`SiteQuotaRow.tsx` 已有提示位）；若承载不了则退化为"表单末尾独立分区 + 明确文案"，并在提交信息里说明原因。
  - 高级配置内部去除三组混排，只留「Codex 私有能力」与「本站覆盖请求头」。
- i18n 双份 + 相关前端测试（`SiteFormModal.test.tsx` 若存在）。

验证：`pnpm typecheck` + `pnpm test:run`；手动：新建站点不选协议 → 保存 → 确认协议被自动写入；手动改协议后再保存，确认不被覆盖。

### S5 MCP 检查更新可用化（R4）

改动点：
- 移植 `src-tauri/src/adapters/mcp_version.rs` 的 args 感知解析到 `commands/mcp_update.rs`（覆盖 `npx` / `uvx`），保留原函数作兜底。
- 新增 `mcp_update_state` 表（**不加入指纹清单**），版本结果改写到该表；`mcp_servers` 的三个版本列保留但停止写入。
- npm 子进程加超时，避免按钮永久挂住。
- `src/pages/McpPage.tsx`：解析不到的条目显示"不支持检查更新"（新增 i18n 键）；进页面自动检查改为带 TTL 的按需触发（**先确认 perf 任务是否已改这块，避免重复**）。

验证：
- 单测：`npx` + args 形态能解析出包名；`uvx` 形态能解析；不可解析条目返回明确标记。
- **指纹回归**：触发一次检查后，`compute_logical_fingerprint` 不变。

### S6 三个既有缺陷（R5，**最后做**）

改动点：
- `src-tauri/src/models_fetch/mod.rs`：删除尝试数组中重复的 `(Bearer, true)`（`:239-243` 中与方法前 OpenAI 伪装重复的那一项）。**动手前先 `git log --oneline -5 -- src-tauri/src/models_fetch/mod.rs` 确认 perf 任务已落地该函数的改造**，若对方已重构则在其新结构上改。
- `src-tauri/src/webdav.rs:293-306`：远端剪枝前读 manifest，把 `bundleFileName` 排除在待删候选之外（无 manifest 时按现状）。
- `src-tauri/src/sync.rs:139-141`：注释改为与实现一致（缺字段固定按 `LEGACY_FINGERPRINT_ALGORITHM_VERSION` 处理）。

验证：三处各补单测（重复尝试次数、剪枝豁免指针包、注释无测试）。

## 验证命令

```bash
# Rust（注意前置里的 MSVC PATH 与 CARGO_TARGET_DIR）
cd /e/xiaobai-switch && cargo test --manifest-path src-tauri/Cargo.toml

# 前端
cd /e/xiaobai-switch && pnpm typecheck && pnpm test:run

# 打包（最后一步，出安装包后真机点一遍）
cd /e/xiaobai-switch && export TAURI_SIGNING_PRIVATE_KEY='E:\xiaobai-switch\.updater-private.key' && export TAURI_SIGNING_PRIVATE_KEY_PASSWORD="" && pnpm tauri build --bundles nsis
```

预期：`cargo test` 540+ 全绿；`pnpm test:run` 374 全绿（`validateUpdaterSigningSecret` / `generateUpdaterManifest` 两个文件的收集期失败是本机既有问题，不算回归）；`pnpm typecheck` 干净。

## 风险点 / 回滚

| 风险 | 影响 | 缓解 |
|---|---|---|
| 改了表清单却忘记递增 `FINGERPRINT_ALGORITHM_VERSION` | 新旧版本对同一份数据算出不同指纹 → 互相覆盖（历史事故） | 护栏测试 `sync.rs:711-729` 会拦住；评审时确认版本号已升到 2 |
| 漏掉 `ensure_incremental_schema` 的某个分支 | 老库拿不到新表且不报错 | 每个建表都挂进 `ensure_incremental_schema`，并用一份"旧版本库"样本跑迁移测试 |
| 停止代理的新语义改变用户预期 | 停止后 CLI 被写回直连 | 已在 S2 加界面说明；行为本身是安全方向（不留死端口） |
| S6 与 perf 任务同函数双改 | 冲突 / 覆盖对方改动 | S6 最后做，动手前查 log 确认 |
| 全局请求头为设备本地（设计取舍） | 两台机器需要各配一次 | 已在设计文档说明；确有需要时另立项做方案 B |

回滚：每步独立提交；任一步出问题 `git revert <step>` 即可，无破坏性迁移（不删列、不改指纹版本、站点级请求头数据原地保留）。

## 提交前检查

- [ ] `git diff --cached --stat` 里没有并行会话的文件
- [ ] `cargo test`、`pnpm test:run`、`pnpm typecheck` 全绿
- [ ] `FINGERPRINT_TABLES` 只新增了 `local_proxy_headers`；`FINGERPRINT_ALGORITHM_VERSION` 已递增为 2 且护栏测试同步更新；`mcp_update_state` **未**入清单
- [ ] 新增列/表都挂进了 `ensure_incremental_schema`
- [ ] i18n 双份齐全，界面上没有硬编码中英文
- [ ] 打包出的安装包真机验证：新建站点自动检测、改端口后重启应用、站点覆盖全局请求头三条
