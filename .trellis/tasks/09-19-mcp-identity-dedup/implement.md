# 执行计划

三个可独立验收的里程碑：**M1 防新增**（1–2）、**M2 清存量**（3–4）、**M3 附带修复与全范围核查**（5–6）。
M1 合入后即可阻止新重复产生，M2 之前用户仍可手工删旧行，不存在半成品状态。

## M1 — 身份原语与判定收紧

- [x] **1. 新模块 `src-tauri/src/adapters/mcp_identity.rs`**
  - `CLIENT_PRIVATE_FIELDS`、`normalize_command`、`coarse_identity`、`equivalence`。
  - ~~`entry_fingerprint` 改为委托 `equivalence` 的薄封装~~ → **执行时纠正**：两套口径必须分开。
    `entry_fingerprint` 删除（HEAD 上已无生产调用方），接管比对保留原窄口径手写比对，
    并补 `takeover_refuses_when_user_disabled_or_retuned_entry` 钉死。详见 design.md
    「两套口径必须分开（实现阶段的纠正）」。
  - 先写测试再写实现。必须覆盖：
    - `npx` / `npx.cmd` / `D:\Program Files\nodejs\npx.cmd` 归一化相等；
    - Claude 形状（带 `type`/`enabled`/`timeoutMs`）与 Codex 形状等价；
    - args 不同 → 粗身份不同；`url` query 不同 → 粗身份不同；
    - env 值不同 → 粗身份相同但 `equivalence` 不同（正是本机 `ssh` 那对的形状）；
    - 身份串里不得出现任何 env/headers 明文值。
  - 验证：`cargo test mcp_identity` ✅ 8 个用例

- [x] **2. 四处判定收紧**（`commands/mcp.rs`）
  - `scan_existing_mcp`：`imported_id` 先按粗身份匹配，未命中再按名字兜底。
  - `import_scanned_mcp`：命中粗身份 → 不建行，新增 `already_imported` 结果项（只含 id 与名字，
    **不含 config/env**）；`failed` 语义不变。
  - `save_mcp_server`：新增行命中他行粗身份 → `validation_failed` 且信息点名冲突条目；按 id 编辑自身放行。
  - `apply_to_targets`：同一目标上两条启用且粗身份相同 → 该目标 `ok=false`、两条都不写。
  - 守卫一律放命令层，`repo/*` 保持对 `crate::adapters` 零依赖。
  - 补测：同批次连续纳管同一服务器的两个条目只建一行（钉死 `import_entry` 每条重读 `list_full` 的必要性）。
  - 验证：`cargo test mcp` ✅ 95 passed；`cargo test` ✅ 594 passed / 0 failed；
    `cargo clippy --all-targets` 告警数 134 = 本机 clippy 1.98 基线，**本次新增代码零告警**
    （`-D warnings` 在本机基线即红，属既有存量问题，另案）
  - **回滚点**：M1 全部改动可整体 revert，无数据、无 schema 影响。

## M2 — 存量重复的检测与合并

- [ ] **3. 后端 `detect_mcp_duplicates` / `merge_mcp_servers`**
  - 检测：按粗身份分组，组内 >1 才算一组；`equivalent` = 组内 `equivalence` 是否全等；
    建议保留条排序键固定（有目标绑定 → 启用 → `created_at` 最早）。
  - 合并：非等价组未显式给 `keep_id` → 整批拒绝；先 `create_local_backup(.., "pre_mcp_merge", ..)`；
    组内重读复核粗身份（防同步换库插队）；`targets` 取并集、`enabled` 取任一启用；删 drop 行；
    最后统一 `apply_to_targets` 扫掉陈旧 `xiaobai_<name>`。
  - 注册到 `lib.rs` 的命令表。
  - 必测：合并后只剩一行且 `targets` 为并集；被丢弃行的托管键从目标客户端消失；
    非等价未指定保留条 → 库与客户端文件**零改动**（对比合并前后字节）；
    合并前确实产出 `pre_mcp_merge` 快照；合并中途失败不删行。
  - 验证：`cd src-tauri && cargo test mcp`

- [ ] **4. 前端**（`McpPage.tsx` + `stores/mcpStore.ts` + `types/mcp.ts` + `lib/browserMock.ts` + i18n）
  - 横幅：`McpPage.tsx:892` 前插入 `Alert type="warning" showIcon`（形状抄 `ProxyPage.tsx:340-345`；
    **不要**抄 `RulesPage.tsx:173-189` 的 `title=`，antd Alert 不认）。
  - 合并对话框：等价组一键合并并显示预选保留条；非等价组并排差异 + 必须手动点选保留条才能提交。
  - 纳管卡片：「已纳管」Tag 带上库内名字；`already_imported` 计入结果提示。
  - mock 层补两个新命令的 `case` 与 fixture（`browserMock.ts` 未知命令会 throw）。
  - i18n：`zh-CN.json` + `en-US.json` 同步加 `mcp.dup*` 键，两侧键必须成对。
  - 验证：`pnpm typecheck && pnpm test:run`
  - **回滚点**：M2 前端与后端命令可整体 revert；已执行过的合并靠 `pre_mcp_merge` 快照回退。

## M3 — 附带修复与全范围核查

- [ ] **5. `adapters/mcp_version.rs`**
  - `command == "npx"` 等字面比较改用 `normalize_command`；补一个绝对路径 `npx.cmd` 能被识别的用例。
  - 验证：`cd src-tauri && cargo test mcp_version`

- [ ] **6. 全范围核查**
  - `cd src-tauri && cargo test && cargo clippy --all-targets -- -D warnings`
  - `pnpm typecheck && pnpm test:run`
  - 确认 `sync.rs` 的 `FINGERPRINT_TABLES` / `FINGERPRINT_ALGORITHM_VERSION` 钉死断言仍通过（表结构未动）。
  - 更新 `.trellis/spec/backend/` 相应条目（MCP 身份口径属跨机协议级知识）。

## 真机验证（M1–M2 完成后）

- [ ] 打开 MCP 面板：本机现有 7 行应报出 **2 组**重复，`sequential-thinking` 组判「等价」、
      `ssh` 组判「非等价」（args 真不同：一条带 `--whitelist`）。
- [ ] 扫描：Codex 的 5 条手工条目全部显示「已纳管为 <库内名>」，不再提供纳管入口。
- [ ] 等价组合并一次，确认 `~/.codex/config.toml` 未受影响（未勾目标时不该写盘）。
- [ ] 勾选目标并应用，确认同一服务器在客户端里**只有一条** `xiaobai_*`。
- [ ] 触发一次 WebDAV 同步（上传→另一台或换库路径），确认换库后检测照常、无自动合并、无删行。

## 交付说明必须写明

- 合并前会自动落 `pre_mcp_merge` 整库快照，可用既有一键恢复回退。
- 非等价重复不会自动合并，需用户点选保留条。- 安全提醒：`ssh` 条目的密码写在 `args` 里，`config_json` 在库中**不加密**、
  `~/.codex/config.toml` 里也是明文；建议挪到 `env`（该列走 AES-GCM）。属既有设计，本任务不改。

## 提交归属备注（2026-09-19，共享索引竞态）

同一工作树、同一分支上有**另一个并发会话**（`.trellis/tasks/09-19-release-0-1-6`「发布 v0.1.6」）。
它 23:06 的 `git commit` 把我 stage 的 M1 文件一起卷进了 `chore(version): bump version to v0.1.6`。
该会话随后自行 `reset --soft HEAD~1` 拆开，只重提了自己的 4 个版本号文件，M1 退回索引；
本任务随后用**显式 pathspec** 单独提交了 M1。全程无内容丢失（拆前后逐字节比对一致）。

**教训（已进项目记忆）**：多个会话共享同一个 git 索引，`git commit` 不带 pathspec 就会把别人
stage 的改动一起提交。以后一律 `git commit -m … -- <本次要提交的具体路径>`。


