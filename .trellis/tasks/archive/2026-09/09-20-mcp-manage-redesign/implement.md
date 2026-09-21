# MCP 重设计执行计划（implement.md）· 单列表方向

> 上一轮（三页签方向）已上线但用户全面不满意，本轮按新原型 `prototypes/mcp-unified-manage.html` 重做为单列表。以下为本轮计划。

## 原型（已完成）

- [x] 新原型 `prototypes/mcp-unified-manage.html` 建成并多轮评审迭代（单列表 / 卡片四开关 / 自动扫描内联 / 仓库搜索 / 冲突两选），已提交 `ecf07c1`。用户 2026-09-21 逐点确认方向。

## 实现批次（前端为主，后端零改动）

### B1 · 主页面单列表骨架（R1）
- [x] `McpPage.tsx` 去 `Tabs`，改为单列表容器；顶栏精简（搜索 + 检查更新 + 一键更新 + 添加 MCP），删客户端筛选条与图例行。
- [x] `SavedMcpList.tsx` → `McpCard`：名称/类型/命令预览/总开关/编辑删除/更新提示。
- [x] `ApplyPanel.tsx` 退役（应用改为卡片即时触发）；`ApplyPanel.tsx` + `SavedMcpList.tsx` 已删除。

### B2 · 卡片逐客户端即时开关（R2）
- [x] `McpCard` 加四客户端开关行；点击 = 改该 server `targets` → `saveServer` → 回显 sweep（失败才弹结果）。
- [x] 总开关（enabled）沿用现有 `handleToggleEnabled` 逻辑；`busyToggle` 防连点、逐卡独立 spinner。

### B3 · 自动扫描 + 未纳管内联（R3）
- [x] mount 的 `scan_existing_mcp` 结果分流：托管/已纳管/冲突排除；未纳管非冲突 → `UnmanagedMcpCard`（虚线边框 + 「未纳管·在 X」+ 纳管）。
- [x] 就地纳管 / 全部纳管 → `import_scanned_mcp`；删独立「扫描纳管」页签入口；save/delete/toggle 后 `runScan()` 刷新。

### B4 · 添加弹窗两来源 + 仓库搜索（R4）
- [x] 添加弹窗 `Segmented` 两来源（手动 / 从仓库安装）；手动表单 env/headers/其他收进「高级配置」折叠；编辑/带草稿时不显示来源切换。
- [x] 仓库安装：进入 `discover_mcp_registry` 推荐；搜索 `search_mcp_registry` + cursor「加载更多」；只看本地开关 + 来源提示文案保留。

### B5 · 冲突对比弹窗两选（R5）
- [x] `ConflictModal`：左右对比 config + 密钥键名（值不出后端）；两选「用客户端的（反向入库）/ 该客户端先跳过」，无「用我的覆盖」。
- [x] 「反向入库」= `importScanned([{target,key}])`，后端撞重名如实报错供改名重试；跳过 = 关弹窗、不改库。

### B6 · i18n + 测试 + 兼容验证（R6）
- [x] `mcp.` 文案 zh/en 双边同步（新增 addSource*/clientToggle/conflict*/unmanaged* 等，双边零漂移）；沿用既有 `browserMock.ts` 调用路径（无新命令）。
- [x] 重写 `McpPage.test.tsx`（26 例，覆盖四开关/纳管/仓库/冲突）；`pnpm typecheck` 通过、`pnpm test:run` 441 例通过（2 个失败为既有 updater 签名测试的编码错误，与本任务无关，已 stash 复现证实）。
- [ ] 存量数据与四目标已应用配置抽查无损（上线前真机）；后端零改动 → `mcp_servers` 指纹/schema 未变。

## 验证命令

- 前端：`pnpm typecheck`、`pnpm test:run`。
- 后端（仅当最终走 design.md 方案 B 才需）：`src-tauri` 下 `cargo test`；`FINGERPRINT_TABLES` 复查。

## 风险文件 / 回滚点

- 本轮**不改**后端高风险文件（`mcp_identity.rs` / `repo/mcp.rs` / `adapters/mcp.rs`）——若实现中发现必须改，停下重新评估并回规划。
- 回滚点：前端重构单批次 commit 可 revert；实现前备份 `~/.xiaobai-switch`；上线前备份四目标 MCP 配置片段。

## task.py start 前检查

- [ ] 新原型已评审确认（已完成，2026-09-21）。
- [ ] 用户已批准本轮最终规划（1.4 review gate）。
