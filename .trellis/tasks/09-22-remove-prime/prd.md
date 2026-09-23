# 移除 Prime 应用目标

## Goal

彻底移除 Prime 应用目标（与 Claude Code / Codex / Pi 并列的第四个目标）。移除后：老用户的库升级后仍能读写、本应用当年写进用户机器 `~/.prime` 的托管痕迹被收口清理；应用中心、MCP、全局约束、代理、托盘等处不再出现 Prime。

（原任务含"新增 OpenCode"，用户已决定 OpenCode 暂不做，本任务只做 Prime 移除。）

## Background / 确认事实（代码勘查 + 规范）

- **Prime 已落库，不能直接删枚举**。`TargetKind` 被 serde 序列化进多处用户数据库持久化 JSON：`settings` blob、`mcp_servers.targets_json`、`agent_rules.targets_json`、`sync_meta` 的两份 applied 集合。`TargetKind` 未开 `#[serde(other)]`，**先删 `TargetKind::Prime` 变体会让老用户整份文档反序列化失败 → 打不开数据库**（`.trellis/spec/backend/target-retirement.md` §4 Bad case 1）。
- **有正式规范与先例**：退役流程见 `.trellis/spec/backend/target-retirement.md`；worked example 是 `src-tauri/src/zcode_retirement.rs`（ZCode 目标退役，27 例守门测试）。本任务照该模板做。
- **两种拼写都要认**：`TargetKind` 的 `#[serde(rename_all="snake_case")]` 让 `Prime` 落库为 `"prime"`；手写 `as_str()` 也产出 `"prime"`；另有历史 alias `"prime_agent"` / `"prime-agent"`。清洗器需认全 `["prime","prime_agent","prime-agent"]`。
- **Prime 全足迹已勘查**（约 40 处）：整文件 `adapters/prime.rs`、`components/apply/PrimeApplyPanel.tsx(.test)`；其余为枚举变体/match arm/字段/UI/i18n/mock/测试/文档的局部删除。清单见本任务 design.md。
- **列不物理删**：Prime 无独占数据库列（设置与 targets 都在 JSON blob / targets_json）；无需动 DDL。`sync.rs` 的 `FINGERPRINT_TABLES` 不含 Prime 专属表，无需改列集合。

## Requirements

R1. 先落地数据清洗层（硬顺序第一步）
- 新增自包含模块 `src-tauri/src/prime_retirement.rs`（镜像 `zcode_retirement.rs`）：
  - `ensure_in_db(&Connection) -> AppResult<DbChanges>`：清 `settings` blob 里的 `primeAgentDirOverride` 等键与 `localProxyTargets` 中的 prime、`mcp_servers.targets_json` / `agent_rules.targets_json` 中的 prime 元素、`sync_meta` 两份 applied 集合中的 prime。幂等。挂 `db/migrate.rs::ensure_incremental_schema` 末尾，**不**升 `SCHEMA_VERSION`。
  - `leftover_home_override(&Connection) -> AppResult<Option<String>>`：从原始 settings JSON 读回历史 `primeAgentDirOverride`（结构体字段删除后仍留在 blob）。
  - `clean_external_once(Option<&str>) -> FileReport`：清用户机器 `~/.prime`（或 override 目录）里我们写的 `xiaobai_` MCP 条目、全局约束托管块、`xiaobai_` provider 等；块外/用户内容逐字节保留（含 BOM）；畸形文件原样保留。挂 `lib.rs` setup（`AppState::init()` 之后、托盘/本地代理之前）。
- 清洗器**自包含**：只认字符串字面量，不 import 被删的 `prime` 适配器、不引用 `TargetKind::Prime`（否则 R3 删完它自己编译不过）。
- 错误处理按 spec §4：清洗计划为空则不写不备份；出错降级为 `warn` 下次重试；空白 override 不回退真实 `~/.prime`；同一根 canonicalize 去重只清一次。

R2. 守门测试先红后绿（随 R1）
- 照 spec §6：两种拼写各造夹具（JSON 数组元素 + 行表列值两条通道）；覆盖幂等+只备份一次、未知目标跳过而非冒充、托管条目被摘而用户条目存活、畸形文件原样保留、无痕迹不写盘、空白 override 不回退、历史 override 键清洗后仍可读回、同一根只清一次。
- 变异验证至少：托管前缀判断去掉 / 清理改 no-op / 备份挪到写盘之后 / 缺 END 从 Err 改成截断，确认对应用例变红。

R3. 删除 Prime 代码（硬顺序最后一步，与 R1/R2 同批发布）
- 整文件删：`src-tauri/src/adapters/prime.rs`、`src/components/apply/PrimeApplyPanel.tsx`、`PrimeApplyPanel.test.tsx`。
- 后端局部删（枚举变体/arm/字段/注册）：`domain/mod.rs`（TargetKind Prime 变体+as_str/parse arm、`PrimeApplyOptions`、`prime_agent_dir_override` 字段+Default）、`paths.rs`（`default_prime_agent_dir`/`resolve_prime_agent_dir`/`PRIME_AGENT_CODING_AGENT_DIR`+测试）、`adapters/{mod,mcp,mcp_scan,agent_rules,agent_update,atomic}.rs`、`commands/{apply,mcp,rules,thinking,targets,skills,sites,settings}.rs`、`key_switch.rs`、`route_switch.rs`、`local_proxy/{mod,routing}.rs`、`cli_detect.rs`、`backup.rs`、`tray.rs`、`tray_apply.rs`、`domain/thinking.rs`、`repo/{rules,thinking,mcp}.rs` 测试夹具。
- 前端局部删：`types/{domain,mcp,agent}.ts`、`stores/{uiStore,applyStore,settingsStore}.ts`、`components/apply/{ApplySidebar,ApplyPage,ApplyFooter,hydrateApplyForm,showApplyOutcome,TargetStatusCard}` 及其测试、`components/skills/SkillTargetIcon.tsx`、`pages/{mcp/targets.ts,RulesPage,ProxyPage,SettingsPage}` 及测试、`pages/McpPage.test.tsx` 夹具、`lib/{thinkingPreset,browserMock}.ts`、`stores/proxyStore.test.ts`。
- match 穷尽：删 arm 后 `TargetKind`/`ScanTarget`/`SkillTarget` 的所有 match 仍需穷尽（Rust 编译器会逼出遗漏）；前端目标→i18n key 映射用 `never` 兜底，不得给默认臂指向存活目标。
- i18n：`zh-CN.json` / `en-US.json` 成对删除所有 `prime*` 键（`apply.targetPrime`/`resultPrimeOk`/`groupPrimeModels`/`primeWriteAllModels(+Hint)`/`removePrime*`/`thinkingPrimeAnthropicNoExtended`、`settings.primeAgentDir*`、`skills.target.prime`、`mcp.targetPrime`、`rules.targetPrime`×2、`proxy.targetPrime`×2、`sites.goApplyPrime`）；宣传文案里的 "Prime" 字样删除。

R4. 文档同步
- `AGENTS.md`：删所有 Prime 段落（目标清单、命名空间、明文 key 落点、MCP 目标表 Prime 行、全局约束落点表 Prime 行、锁约定、图标说明、Prime 关键字段、Prime 合并规则与配置目录解析）。
- `README.md` / `README_EN.md`：删 Prime 落点与 `PRIME_AGENT_CODING_AGENT_DIR` 相关行。

## Acceptance Criteria

- AC1（老库可读可存）：以 v0.1.6 形状（settings blob 含 `primeAgentDirOverride`、某 MCP/agent_rules targets 含 `prime`、sync_meta applied 含 prime）的库升级后，`prime_retirement::ensure_in_db` 幂等清洗，设置页可读可存、MCP 列表读得出、其余目标的全局约束托管块与 MCP 托管条目仍能正常清理。
- AC2（用户机器痕迹收口）：用户机器 `~/.prime`（或 override）里我们写的 `xiaobai_` MCP 条目 / 全局约束托管块被摘除，块外与用户自有条目逐字节保留（含 BOM）；畸形文件原样保留；从未用过 Prime 者全程 no-op、不写盘。
- AC3（残留扫描）：全仓库（排除 `.trellis/`、`prime_retirement.rs` 内的退役字面量、`.mimosa/`）无 `Prime`/`prime` 作为**存活应用目标**的引用；`TargetKind`/`ScanTarget`/`SkillTarget` 无 Prime 变体。
- AC4：`cargo test`（含新增退役守门测试与变异验证）通过；无编译错误、match 穷尽。
- AC5：`pnpm typecheck` 与 `pnpm test:run` 通过；应用中心、MCP、全局约束、代理页、设置页不再出现 Prime。
- AC6：AGENTS.md / README 文字与代码一致，无残留 Prime 落点描述。

## Out of Scope

- 不新增 OpenCode（用户已决定暂不做）。
- 不删除用户机器上的 `~/.prime` 目录或非本应用写入的内容（只摘我们写的托管块/条目）。
- 不改数据库 DDL / `FINGERPRINT_TABLES` 列集合。
- 不动 MCP 统一管理模型（那是后续独立任务 09-22-mcp per-agent 重构）。
