# XiaoBaiSwitch Plus — Agent / 自动化约束

本文档约束人类与编码 Agent 如何改动本仓库。

## 产品规则

- 产品名：**XiaoBaiSwitch Plus**（`com.github.licoy.xiaobai-switch.plus`）；本仓库是 Licoy 的 XiaoBaiSwitch 的 fork，原始版权、作者署名与 MIT 归属原作者。
- 展示名与二进制名不同：窗口/安装包/productName 用 `XiaoBaiSwitch Plus`，二进制用 `XiaoBaiSwitchPlus`（不带空格）。
- 下载与自动更新**只指向我们自己的 GitHub Releases**（`doMySelfZy/xiaobai-switch-plus`），没有官网，也不在 Gitee 发布；更新签名公钥保持现有值，绝不改回作者的公钥。
- 领域单一事实来源（SSOT）是 **站点优先**：Base URL + API key → 模型 → 目标私有能力预设 → 应用到目标
- 目标：Claude Code（`~/.claude/settings.json`）、Codex（`~/.codex/` + 环境变量注入）、Pi（`~/.pi/agent/{models,auth,settings}.json`）与 Prime（`~/.prime/agent/{models,auth,settings}.json`）
- 应用数据根目录：`~/.xiaobai-switch/`（不是 Tauri 的 `app_data_dir`）

## 目录布局

```
~/.xiaobai-switch/
├── xiaobai-switch.db   # SQLite 应用状态
├── master.key          # AES-256-GCM 主密钥（Unix 上权限 0600）
└── backups/            # 应用前备份
```

- **禁止**把用户数据路径写死成 Tauri `app_data_dir`、bundle id 版本字符串，或平台应用支持目录。
- **数据目录迁移 / 接管**：启动时若 `~/.xiaobai-switch` 不存在而 `~/.any-switch`（AnySwitch 时期目录）存在，会复制迁移并校验数据库 sha256 / `master.key`。两个目录同时存在时按**数据库最新修改时间**判断：旧目录明显更新（> 2 秒）才先备份现有新目录为 `.pre-adopt-<unix秒>` 再复制接管，否则保持使用新目录。**任何情况下都不删除旧目录或 pre-adopt 备份**。数据目录覆盖变量优先 `XIAOBAI_SWITCH_DATA_DIR`，并兼容 `ANY_SWITCH_DATA_DIR`。
- **兼容红线（改了会破坏既有数据，只做“新旧双识别”，绝不改协议值）**：
  - WebDAV 同步 manifest 文件名 `xiaobai-switch-sync.json`；
  - WebDAV 远端目录默认值 `xiaobai-switch`（与 manifest 是同一套跨机协议路径）；
  - 本地备份前缀：新文件用 `xiaobai-switch-backup-`，AnySwitch 时期的 `any-switch-backup-` 必须继续识别（列表/恢复/清理/远端保留都要接受两种前缀，排序需先剥离前缀再按时间戳比较）；
  - 备份包内数据库条目名 `xiaobai-switch.db`（内部归档协议，保留才能旧备份可恢复 + 新备份可被旧版读取）；
  - 已应用配置命名空间 `xiaobai_`：Pi/Prime provider id、Codex provider id 派生、`XIAOBAI_SITE_*` 环境变量、`__xiaobai_missing__`、**MCP 服务器名（写入客户端 `mcpServers` / `[mcp_servers.*]` 的键）**；
  - 应用/备份痕迹识别文件名：`xiaobai-model-catalog.json`（Codex 模型目录）、`xiaobai-backup.json`（目标备份元数据）、`.xiaobai-skill.json` 与 `.xiaobai-skill-` 临时前缀（技能安装清单）、`atomic.rs` 的 `.xiaobai-` 临时文件前缀；
  - 深链解析继续接受 `anyswitch:` / `xiaobaiswitch:` 旧 scheme（写链接时用 `xiaobaiswitchplus:`）；
  - 更新签名公钥保持当前值；
  - `restore_official` / 孤儿清理的识别逻辑不得改动。
- API key 在数据库中加密存储；**应用（Apply）** 之后，它们可能以明文出现在 `~/.claude` / `~/.codex` / `~/.pi/agent/auth.json` / `~/.prime/agent/auth.json` / `codex.env` / shell rc 中 —— 须在 UI 文案中说明这一点。

## MCP 统一管控

一份 MCP 定义存在应用数据库中，可勾选应用到 Claude Code / Codex / Pi / Prime，并按客户端分别写入。**每个客户端只写它自己的原生位置**：

| 目标 | 文件 | 结构 |
|------|------|------|
| Claude Code | `~/.claude.json`（设 `CLAUDE_CONFIG_DIR` 时为该目录内同名文件；应用内覆盖同样映射到 `<覆盖目录>/.claude.json`） | 顶层 `mcpServers` |
| Codex | `~/.codex/config.toml` | `[mcp_servers.*]`，用 `toml_edit` 保留注释与其他段 |
| Pi | `<pi agent dir>/mcp.json` | `mcpServers` |
| Prime | `<prime agent dir>/settings.json` | `mcpServers` |

- **Claude Code 的 MCP 不在 `~/.claude/settings.json` 里** —— 该文件的 `mcpServers` 会被 Claude Code 忽略，写入必须落到 `~/.claude.json`。
- 托管条目 = `xiaobai_<name>`。应用时只清理该前缀下已不在当前清单里的条目；改名、禁用、删除、改绑目标后都会触发清理。**不要**去动用户自己的 MCP 条目。
- 写入前先备份、再原子替换，并用 `<文件名>.lock` 目录与目标 CLI 自己的写入互斥（Prime 的 `settings.json` 与 prime 适配器共用同一把锁）。
- 既有配置形状不合法时（如 `mcpServers` 不是对象、Codex `mcp_servers` 不是表）必须**报错并原样保留文件**，不得覆盖。
- `env` 与 `headers` 在数据库里加密存储，应用后以明文落到客户端配置；`config` 原样透传（因此不要把密钥写进 `config`）。UI 须同时说明这两点。
- MCP 数据随应用数据库走既有 WebDAV/备份同步，`mcp_servers` 必须留在 `sync.rs` 的 `FINGERPRINT_TABLES` 里，否则 MCP 变更不会被判定为数据变更、永远不发同步。

### 已有 MCP 的扫描与纳管

用户可以把手工配过的 MCP 纳管进来（`adapters/mcp_scan.rs` + `scan_existing_mcp` / `import_scanned_mcp`）。硬约束：

- **扫描只回元数据**：`ScannedMcp` 只带密钥**键名**（`env_keys` / `header_keys`），绝不带值。密钥值只在纳管时由后端按 `(target, key)` 定位符重新读盘取得，**不经过前端**；`import_scanned_mcp` 也只回元数据（`repo::mcp::save` 的返回值含明文，不得回传）。返回值携带 `apply: Option<McpApplyResult>`（本次接管写盘结果，只含 target/ok/backup_paths/message，无密钥）。
- **纳管即接管，避免重复加载**：一份 MCP 若按粗身份（`kind + config`）存在于多个客户端，纳管**只建一条记录**，其 `targets` 取所有同款客户端的并集、全部启用（`scan_target_union`）；`import_scanned_mcp` 入库后**立即**对这些客户端复用应用路径（`apply_to_targets`）写盘接管，并把这些目标记进 `applied_targets`——删除该记录时才能把所有关联客户端里的 `xiaobai_<name>` 一起清干净。接管的等价判定：若目标里存在同名的**未托管**条目，判断它是否仍与库内记录等价（忽略 `type`/`transport` 这类显式类型标记）：
  - 等价 → 删除它再写 `xiaobai_<name>`，净结果仍是一条；
  - 不等价（用户扫描后又改过）→ **报错并跳过该目标**，文件原样保留，绝不用库里的旧版本覆盖用户的改动（失败目标记在返回的 `apply.results` 里，前端据此提示）。

  漏掉接管会让客户端同时加载两份同一个 MCP。Codex 侧用 `codex_entry_to_json` / `codex_record_to_json` 把 `http_headers` 与 `env` 子表规约成同一形态，两个方向共用一套口径。


## 全局约束（Agent 指令统一注入）

一段用户级 Markdown 约束，勾选目标后由应用分别写入各 CLI 自己的全局指令文件。**每个客户端只写它自己的原生位置**：

| 目标 | 文件 | 备注 |
|------|------|------|
| Claude Code | `${CLAUDE_CONFIG_DIR:-~/.claude}/CLAUDE.md`（应用内覆盖优先） | 单文件上限 4 MiB；解析走 `paths::claude_rules_path` |
| Codex | `${CODEX_HOME:-~/.codex}/AGENTS.md` | 同目录的 `AGENTS.override.md` 会**整体遮蔽**它：只检测并在 UI 警告，绝不改写 override 文件 |
| Pi | `<pi agent dir>/AGENTS.md`；若目录内只有用户的 `CLAUDE.md` 则追加到它 | 不新建 `AGENTS.md` 去压过用户自己的 `CLAUDE.md` |
| Prime | `<prime agent dir>/AGENTS.md`；选择规则同 Pi | Prime 文档未提 `AGENTS.override.md`，按不支持处理 |

- 采用**托管块**而不是独占整个文件：块外用成对标记 `<!-- xiaobai-switch:begin global-rules -->` … `<!-- xiaobai-switch:end global-rules -->` 界定，块外用户内容必须逐字节保留（含 UTF-8 BOM）。标记前缀与 `xiaobai_` 命名空间同源，属兼容红线的一部分。
- 清理时若文件只剩空白（整个文件都是本应用写的）就**删除该文件** —— 否则空的 `AGENTS.md` 会反过来遮蔽用户自己的 `CLAUDE.md`。唯一允许的字节变化是尾部空行归一为一个换行。
- 正文含标记字面量、文件只有 BEGIN 没有 END、路径是目录、文件非 UTF-8：一律**报错并原样保留文件**。
- 单条正文存在 `agent_rules` 单行表（`id = 1`）；「已应用目标」用 `sync_meta` 的 `agent_rules_applied_targets`，写入目标 = 本次勾选 ∪ 上次写过（取消勾选 / 清空正文时靠它清理）。空正文或空目标 = 不生效。
- 写盘前备份到 `~/.xiaobai-switch/backups/agent-rules/`，原子替换 + `<文件名>.lock` 互斥；内容无变化时**不备份、不写盘**。
- `agent_rules` 必须留在 `sync.rs` 的 `FINGERPRINT_TABLES` 里，否则约束变更永远不发同步。
- 约束正文以明文写入上述文件，UI 须说明这一点；写入的提示行固定双语（不随界面语言变）。

## UI Shell

### 标题栏

- 高度 **36px**（对齐 macOS 红绿灯）。
- 自定义标题栏：`titleBarStyle: Overlay` + `hiddenTitle: true`。
- 拖拽区域：
  - CSS：`.title-bar-drag` → `-webkit-app-region: drag`
  - 可交互子元素：`.title-bar-nodrag` + 按钮 / `.ant-dropdown-trigger` → `no-drag`
  - macOS：标题栏还需设置 `data-tauri-drag-region`
  - 始终在 mousedown 时调用 `getCurrentWindow().startDragging()`（回退方案；CSS 拖拽单独失效时必须有它）
- Windows：左侧内边距约 12；macOS：左侧内边距约 72，给红绿灯留位
- 设置开关放在标题栏；设置页打开时，显示关闭（XCircle）状态

### 主侧栏

- 宽度 **48px**；仅图标的圆形按钮 **36×36**，`borderRadius: 50%`
- 标签用 antd `Tooltip`，`placement="right"` —— **图标下方不要文字**
- 激活态：`token.colorPrimaryBg` + `token.colorPrimary`
- 进入设置页时整栏隐藏

### 设置页

- 左侧 **设置侧栏**（`w-56`）+ 右侧内容（`colorBgElevated`）
- 侧栏：返回行（ArrowLeft + Esc 提示）+ antd `Menu` `mode="inline"`，带分区图标
- 内容：用 `SettingsGroup` 分组（Card、柔和边框、分区标题在卡片上方）
- Esc 退出设置，回到主页面
- 在 UX 允许时，优先在开关/选择时即时 `saveSettings`，而不是一个统一的批量保存表单

### 应用中心

- 与设置页同一套壳：左侧 **目标侧栏**（`w-56`）+ 右侧内容（`colorBgElevated`）
- 当前目标为 **Claude Code**、**Codex**、**Pi** 与 **Prime**；Claude/Codex/Pi 图标来自 `@lobehub/icons`，Prime 使用 lucide `Sparkles`（`@lobehub/icons` 无 Prime 图标）
- 每个目标有各自的专用表单（不是共享的双目标复选框面板）
- Claude 关键字段：鉴权 key 风格、默认模型、opus/sonnet/haiku 别名映射、effort 等级
- Codex 关键字段：默认模型、写入全部模型目录开关、reasoning effort；平台能力默认跟随站点 `codex-compact` / `codex-vision` / `codex-imagegen` / `codex-search`，也可在应用中心自定义覆盖
- Pi 关键字段：默认模型、写入站点全部模型开关、协议说明、逐模型思考能力与默认思考等级；思考预设按 `(site_id, target)` 记忆，不根据模型名称猜测
- Prime 关键字段：默认模型、写入站点全部模型开关、协议说明、逐模型思考能力与默认思考等级；配置目录默认 `~/.prime/agent`，不要写到 `~/.pi/agent`
- 站点编辑含默认收起的「高级配置」（连接协议、备注）与「Codex私有能力」；`xiaobaiswitchplus://sites`（并兼容旧 `anyswitch://` / `xiaobaiswitch://`）用同一套 kebab 键导入预设
- 分区卡片复用 `SettingsGroup`，保持视觉语言一致

## Ant Design 约定

### ConfigProvider

始终用 antd `ConfigProvider` + `App`（`AntdApp`）包裹应用。全局 Modal 默认值：

```tsx
<ConfigProvider
  modal={{
    centered: true,
    styles: {
      container: {
        maxHeight: "calc(100vh - 32px)",
        display: "flex",
        flexDirection: "column",
        overflow: "hidden",
      },
      body: { overflowY: "auto", overflowX: "hidden", minHeight: 0 },
    },
  }}
>
  <AntdApp className="h-full">…</AntdApp>
</ConfigProvider>
```

### Message / Modal / notification

- **必须**使用 `App.useApp()` → `const { message, modal, notification } = App.useApp()`
- **禁止**：从 `antd` 静态调用 `message.success()`、`Modal.confirm()`（会破坏 `App` 下的主题 / 上下文）
- 确认框：`modal.confirm({ centered: true, … })`

### Modal 组件

每个表单/对话框 `Modal` 都应：

| 属性 | 值 |
|------|--------|
| `centered` | `true`（或依赖 ConfigProvider） |
| `destroyOnHidden` | `true`（antd 5.23+ / 6.x；优先于已弃用的 `destroyOnClose`） |
| `mask` | `{ enabled: true }` —— **不要开 `blur`**：antd 会因此挂上 `.ant-modal-mask-blur`（整窗 `backdrop-filter: blur(4px)`），在 175% 缩放下是一次全视口模糊，收益不抵开销（2026-09-16 性能任务移除；悬浮窗透明层上的同类模糊同理已删） |
| `width` | 表单优先 `520`–`560` |
| 高度 | 容器 `maxHeight: calc(100vh - 32px)`；标题 / 底栏固定，body 内部滚动 |

```tsx
<Modal
  open={open}
  centered
  destroyOnHidden
  mask={{ enabled: true }}
  width={560}
  onCancel={onClose}
  onOk={handleOk}
>
  …
</Modal>
```

### Modal 内的表单

- 优先使用 antd `Form` + `Form.Item`，`layout="vertical"`
- 必填字段用 `rules={[{ required: true }]}` —— 不要自造校验 UI
- 密码字段：`Input.Password`
- 自由文本输入在安全的情况下使用 `allowClear`

### 主题 Token

- 颜色优先用 `theme.useToken()`，不要写死 hex（品牌色 / Windows 关闭按钮红 `#e81123` 除外）
- 将常用 token 同步到 `documentElement` 上的 CSS 变量（`--border-color`、`--color-bg-*`、`--color-text*`、`--color-primary`）

### 图标

- Shell / 导航使用 Lucide 图标；标题栏尺寸 14，侧栏 16–18
- CSS：`svg.lucide { display: inline-block; vertical-align: -0.125em; }`，以便与 antd 对齐

## 国际化（i18n）

- 所有用户可见文案走 `react-i18next`（`zh-CN` + `en-US`）
- 组件中不要硬编码中文/英文，技术标识除外（模型 id、环境变量名、路径）

## 安全 / 脱敏

- 永远不要记录原始 API key；任何可能回显密钥的日志或错误展示前，先走脱敏辅助函数
- 站点列表与详情展示密钥材料时只用前缀（`keyPrefix`）
- 编辑站点时可通过 `get_site_api_key` 按需解密到密码输入框；默认隐藏，禁止写入日志或长期前端状态

## 前端技术栈

- Tauri 2 + React + TypeScript + Vite + antd + Tailwind v4 + zustand + lucide-react
- 路径别名 `@/` → `src/`
- 非 Tauri 开发用 `src/lib/browserMock.ts` 作为浏览器 mock 层；invoke 契约须与 Rust command 保持同步

## 后端（Rust）

- Command 是唯一的 UI→宿主边界；保持 `#[tauri::command]` 表面小且有类型
- 改写目标 CLI 配置前，先原子写入 + 备份
- Codex 的 `wire_api` 保持为 `responses`；provider id 由 `site.id` 派生
- Pi 直接合并 `models.json`、`auth.json`、`settings.json`：保留注释、其他 Provider、OAuth 与未知字段；只管理 `xiaobai_` 命名空间
- Pi 配置目录按应用设置 → `PI_CODING_AGENT_DIR` → `~/.pi/agent` 解析；`auth.json` 权限为 `0600`
- Prime 直接合并 `~/.prime/agent/{models,auth,settings}.json`：规则与 Pi 相同，只管理 `xiaobai_` 命名空间
- Prime 配置目录按应用设置 → `PRIME_AGENT_CODING_AGENT_DIR` → `~/.prime/agent` 解析；`auth.json` 权限为 `0600`
- 环境注入矩阵（shell rc / user env / file_only）必须与设置保持一致
- 数据库 schema 版本号变更时，`apply_schema` 的**每个**「库已存在」分支都要跑一遍增量补齐（`ensure_incremental_schema`）：`CREATE TABLE IF NOT EXISTS` 不会给已有表补列/补表，漏掉分支会让老库拿不到新表新列且不报错

## 测试

- 前端：`pnpm test:run`、`pnpm typecheck`
- Rust：在 `src-tauri` 中执行 `cargo test`
- UI 或适配器改动后，未跑完相关检查，不得声称「已完成」
