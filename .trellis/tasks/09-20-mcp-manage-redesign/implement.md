# MCP 重设计执行计划（implement.md）

## P0 原型（先行，评审通过才进 P1）

- [x] 在 `prototypes/` 新建 MCP 原型稿，覆盖 R1–R4 信息架构与核心交互（`prototypes/mcp-manage-redesign.html` 已建，src/src-tauri 零改动）。
- [x] 用户评审确认（2026-09-20，四项全按推荐项：三页签 / 应用面板页内常驻 / 冲突默认跳过 / 无目标弹窗问一次；结论已落回 prd D4）；评审意见落回本任务 prd/design，不直接进代码。

## P1 页面布局 + 应用目标流程（R1 + R2）—— 已完成并核验通过（2026-09-20）

- [x] 按区块拆分 `src/pages/McpPage.tsx`，重做布局与操作入口（顶栏精简 + 三页签非激活不挂载；新建 `src/pages/mcp/targets.ts`、`SavedMcpList.tsx`、`ApplyPanel.tsx`）。
- [x] 简化应用目标流程（勾选/清理/报错口径），同步 `mcp.` 文案（zh/en 各 +13 键、删 `mcp.noTargets`，双边零漂移；`src-tauri/` 零改动）。
- [x] 更新 `src/pages/McpPage.test.tsx` 相关用例；`pnpm typecheck` + `McpPage.test.tsx` 29/29 通过（tre-8 只读核验，无阻塞项）。
- [ ] 存量数据与四目标已应用配置抽查无损（上线前真机做；P1 零后端改动，风险极低）。

## P2 扫描纳管 + Registry 安装（R3 + R4）

- [ ] 调整 `mcp_scan.rs` / `mcp_identity.rs` / `import_scanned_mcp` 规则，补 Rust 单测。
- [ ] 优化 Registry 搜索安装链路（`mcp_registry/` + 前端 Registry 卡）。
- [ ] `cargo test`（`src-tauri`）+ 前端相关测试通过；同步指纹（`FINGERPRINT_TABLES`）复查。
- [ ] 四目标文件抽查 + 旧数据可读验证。

## 验证命令

- 前端：`pnpm typecheck`、`pnpm test:run`（UI/适配器改动后必跑）。
- 后端：`src-tauri` 下 `cargo test`；clippy 只修增量交集，禁止 `cargo fmt`（见 backend 规范）。

## 风险文件 / 回滚点

- 高风险：`src-tauri/src/adapters/mcp_identity.rs`（身份口径牵连去重/接管）、`src-tauri/src/repo/mcp.rs`（schema 动则走增量补齐）。
- 回滚点：P1/P2 各自批次 commit；上线前数据目录与客户端配置备份。

## task.py start 前检查

- [ ] P0 原型已评审确认。
- [ ] `implement.jsonl` / `check.jsonl` 已策展（子代理上下文）。
- [ ] 用户已批准本轮最终规划（1.4 review gate）。
