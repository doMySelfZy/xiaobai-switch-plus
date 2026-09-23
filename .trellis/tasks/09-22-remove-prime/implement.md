# 执行计划：移除 Prime 应用目标

工作流：inline。硬顺序（清洗层先落地→挂载→最后删枚举/UI），一批发布。

## 阶段 A — 数据清洗层 + 守门测试（先红后绿）
1. 通读 `src-tauri/src/zcode_retirement.rs` 与 `.trellis/spec/backend/target-retirement.md`，作为模板与判据。
2. 新增 `src-tauri/src/prime_retirement.rs`：`ensure_in_db` / `leftover_home_override` / `clean_external_once`，字面量 `["prime","prime_agent","prime-agent"]` + `primeAgentDirOverride` + `~/.prime/agent` 落点常量。自包含，不引用 `TargetKind::Prime` / `adapters::prime`。
3. `lib.rs` 加 `mod prime_retirement;`；`db/migrate.rs::ensure_incremental_schema` 末尾调 `ensure_in_db`；`lib.rs` setup（AppState::init 后、托盘前）调 `leftover_home_override` + `clean_external_once`，出错降级 warn。
4. 守门测试（先红）：照 spec §6 造两种拼写夹具（JSON 数组元素 + 行表列值）、覆盖 8 个断言点；变异验证 4 项。此时 `TargetKind::Prime` 仍在，测试应先失败再随实现转绿。
5. `cargo test prime_retirement` 绿。

## 阶段 B — 删除 Prime 代码（编译器/tsc 兜底）
6. 后端整文件删：`adapters/prime.rs`（+ `adapters/mod.rs` 的 `pub mod prime;`）。
7. `domain/mod.rs`：删 `TargetKind::Prime` 变体、`as_str`/`parse` arm、`PrimeApplyOptions`、`prime_agent_dir_override` 字段 + Default。
8. `cargo build`，逐个编译错误消 arm/import/字段：`paths.rs`、`adapters/{mcp,mcp_scan,agent_rules,agent_update,atomic}.rs`、`commands/{apply,mcp,rules,thinking,targets,skills,sites,settings}.rs`、`key_switch.rs`、`route_switch.rs`、`local_proxy/{mod,routing}.rs`、`cli_detect.rs`、`backup.rs`、`tray.rs`、`tray_apply.rs`、`domain/thinking.rs`。注意 `SkillTarget::ALL` 长度 5→4。
9. 删后端 Prime 专属测试 / 夹具（`repo/{rules,thinking,mcp}.rs`、各 `#[cfg(test)]`）。
10. `cargo test` 全绿。

## 阶段 C — 前端删除
11. 整文件删：`components/apply/PrimeApplyPanel.tsx`、`PrimeApplyPanel.test.tsx`。
12. `types/{domain,mcp,agent}.ts` 删 `"prime"`（TargetKind/SkillTarget/ThinkingTarget/ScanTarget 联合、`primeWriteAllModels`、`primeAgentDirOverride`）。
13. `pnpm typecheck`，逐个 tsc 错误消：`stores/{uiStore,applyStore,settingsStore}.ts`、`components/apply/{ApplySidebar,ApplyPage,ApplyFooter,hydrateApplyForm,showApplyOutcome,TargetStatusCard}.tsx`、`components/skills/SkillTargetIcon.tsx`、`pages/{mcp/targets.ts,RulesPage,ProxyPage,SettingsPage}.tsx`、`lib/{thinkingPreset,browserMock}.ts`。目标→i18n key 映射保 `never` 兜底。
14. 删前端 Prime 专属测试断言：`ApplySidebar/ApplyFooter/hydrateApplyForm/showApplyOutcome` test、`McpPage.test.tsx`、`RulesPage/ProxyPage/SettingsPage` test、`thinkingPreset.test.ts`、`proxyStore.test.ts`。
15. i18n 成对删 `prime*` 键 + 宣传文案 Prime 字样（zh-CN / en-US）。
16. `pnpm typecheck` + `pnpm test:run` 全绿。

## 阶段 D — 文档 + 验证
17. `AGENTS.md` 删 Prime 段落；`README.md` / `README_EN.md` 删 Prime 落点与 `PRIME_AGENT_CODING_AGENT_DIR`。
18. 残留 grep（AC3）：`grep -rniE "prime" src src-tauri/src --include=*.rs --include=*.ts --include=*.tsx`，排除 `prime_retirement.rs` 与 `.mimosa/`，确认只剩退役清洗器里的字面量。
19. 最终：`cargo test` + `pnpm typecheck` + `pnpm test:run`。

## 验证命令
```bash
cd src-tauri && cargo test
pnpm typecheck && pnpm test:run
grep -rniE "\bprime\b" src src-tauri/src --include="*.rs" --include="*.ts" --include="*.tsx" | grep -v prime_retirement
```

## 风险 / 回滚点
- **硬顺序**：清洗层没先落地就删枚举 → 老库打不开。阶段 A 必须先完成并留测试。
- **穷尽 match**：删 arm 后编译器/tsc 会报错兜底，别用 `_ =>` 默认臂吞掉。
- **误删用户鉴权键**：脏行解析绝不 `unwrap_or(ClaudeCode)`（spec §4 Bad case 2）；退役清洗只按字面量匹配。
- 回滚：`git revert` 整批。
