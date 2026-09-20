# MCP 重设计技术方案（design.md）

## 架构与边界

- 前端：`src/pages/McpPage.tsx` 拆分为区块组件（顶栏 / Registry / 扫描纳管 / 已保存 / 须知 / 表单 Modal），状态仍经 `mcpStore` / `mcpUpdateStore`，类型以 `src/types/mcp.ts` 为准；新文案一律 `mcp.` 前缀中英双语。
- 后端：command 签名层（`commands/mcp.rs`、`mcp_update.rs`）保持稳定，行为调整收敛在 `adapters/`（`mcp.rs` 写盘与接管、`mcp_scan.rs` 扫描规则、`mcp_identity.rs` 身份口径、`mcp_update.rs` 更新链路）与 `repo/mcp.rs`；`domain/mcp.rs` 类型变更需前后端同步。
- P0 原型只画前端信息架构与交互，不写产品代码、不定后端接口。

## 数据流与契约

- 沿用现有链路：store action → Tauri command → adapter → repo/客户端文件；`save`/`import` 后重拉 `list`；更新检查走 TTL 缓存。
- R2/R3 若调整报错口径或接管规则，只改文案与判定分支，不动四目标原生位置、`xiaobai_` 前缀、备份/原子/锁、非法形状保留等铁律。
- `browserMock.ts` 的 mcp 契约随 command 变更同步更新，否则前端单测失真。

## 兼容与迁移

- `mcp_servers` 表：新增列走增量补齐（每个「库已存在」分支跑 `ensure_incremental_schema`）；不改 `xiaobai_` 命名空间与已落盘值。
- `mcp_servers` 必须留在 `FINGERPRINT_TABLES`；若动表清单则同步递增 `FINGERPRINT_ALGORITHM_VERSION`。
- 存量已应用配置：P1/P2 上线前抽查四目标文件（托管条目存在、用户条目未动）。

## 关键取舍

- 整页重写 vs 渐进重构：选渐进（按区块替换），1333 行单文件一次性重写回归风险高，且 P1/P2 分批天然对应区块。
- 身份口径（`mcp_identity.rs`）调整最敏感：coarse/equivalence 改动会同时影响去重、接管与 `already_imported`，P2 单独给足测试。
- 更新链路（npm/npx/uvx/pip）不动底层安装语义，只优化触发与展示。

## 运维与回滚

- 分批上线：P1、P2 各自独立验证、可独立回滚（git revert 对应批次 commit）。
- 回滚点：P1 前备份 `~/.xiaobai-switch` 数据目录；P2 前备份四目标客户端 MCP 配置片段。
