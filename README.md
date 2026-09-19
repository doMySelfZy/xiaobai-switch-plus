<p align="left">
  <strong>中文</strong> · <a href="./README_EN.md">English</a>
</p>

<p align="center">
  <img src="assets/brand/app-icon-1024.png" alt="XiaoBaiSwitch Plus" width="160" height="160">
</p>

<h1 align="center">XiaoBaiSwitch Plus</h1>

<p align="center">
  <strong>一站式 AI 编码助手配置管理工具</strong><br>
  统一管理 Claude Code、Codex、Pi、Prime 的站点、模型与配置
</p>

<p align="center">
  <a href="https://github.com/doMySelfZy/xiaobai-switch-plus/releases"><img src="https://img.shields.io/github/v/release/doMySelfZy/xiaobai-switch-plus?style=flat-square" alt="Release"></a>
  <a href="https://github.com/doMySelfZy/xiaobai-switch-plus/blob/main/LICENSE"><img src="https://img.shields.io/github/license/doMySelfZy/xiaobai-switch-plus?style=flat-square" alt="License"></a>
</p>

---

> **本项目是 [Licoy](https://github.com/Licoy) 的 [XiaoBaiSwitch](https://github.com/Licoy/xiaobai-switch) 的增强版 fork。**  
> 原作者、原始项目与 MIT 协议归属原作者所有。本仓库在此基础上新增 WebDAV 真同步、MCP 统一管控、全局约束注入等企业级功能，并只从我们自己的 GitHub 发布下载与自动更新。

---

## ✨ 核心特性

### 🎯 站点优先，统一管理
- **一站多模型**：每个站点配置一次 Base URL + API Key，自动拉取所有可用模型
- **目标独立配置**：Claude Code、Codex、Pi、Prime 各有专属表单，互不干扰
- **一键应用**：勾选目标后自动写入各 CLI 的原生配置文件，保持格式与注释

### 🔄 WebDAV 真同步
- **多设备协同**：笔记本、台式机共享一份配置，改完即同步、换机即拉取
- **智能冲突处理**：自动检测指纹版本，识别本地/远程哪边更新，冲突时明确提示
- **安全可靠**：同步前自动备份，数据库 + 主密钥一起校验，不会静默覆盖

### 🔌 MCP 统一管控
- **一份定义，多处应用**：编辑一次 MCP 配置，勾选目标后分别写入 Claude Code、Codex、Pi、Prime
- **托管命名空间**：只管理 `xiaobai_*` 前缀的条目，不干扰用户手工配置的 MCP
- **扫描与纳管**：可将已有的手工 MCP 配置导入管理，密钥不经过前端

### 📝 全局约束注入
- **统一 Agent 指令**：编写一次 Markdown 约束，选择性写入各目标的 `AGENTS.md` / `CLAUDE.md`
- **托管块机制**：用标记界定托管内容，块外用户内容逐字节保留
- **自动清理**：取消勾选或清空正文时，自动移除对应目标的托管块

### 🔐 安全与备份
- **加密存储**：API Key 与 MCP 密钥在数据库中 AES-256-GCM 加密，主密钥权限 `0600`
- **应用前备份**：每次写入目标配置前自动备份到 `~/.xiaobai-switch/backups/`
- **恢复官方配置**：一键清理托管条目，恢复 CLI 原始状态

### 🔗 深链导入
- 支持 `xiaobaiswitchplus://` 协议一键导入站点预设
- 兼容旧版 `anyswitch://` 与 `xiaobaiswitch://` 分享链接

---

## 📥 下载与更新

- **GitHub Releases**：<https://github.com/doMySelfZy/xiaobai-switch-plus/releases>
- **自动更新**：应用内检测 `latest.json`，增量下载补丁
- **无官网、不走 Gitee**

---

## 🚀 快速开始

### 安装

1. 从 [Releases](https://github.com/doMySelfZy/xiaobai-switch-plus/releases) 下载对应平台的安装包
2. Windows：运行 `.exe` 安装程序；macOS：拖入 Applications；Linux：按发行版指引安装
3. 首次启动会在 `~/.xiaobai-switch/` 创建数据目录（若有旧版 `~/.any-switch/` 会自动接管）

### 基本使用

1. **添加站点**
   - 点击「站点」→「添加站点」
   - 填写名称、Base URL、API Key
   - 点击「检测协议」确认连通性
   - 保存后自动拉取模型列表

2. **配置目标**
   - 进入「应用中心」
   - 选择目标（Claude Code / Codex / Pi / Prime）
   - 勾选要使用的站点与模型
   - 配置默认模型、别名映射等
   - 点击「应用」写入目标配置

3. **启用 WebDAV 同步**（可选）
   - 「设置」→「数据同步」
   - 填写 WebDAV 服务器地址、用户名、密码
   - 勾选「启用自动同步」
   - 首次同步会上传本地数据，后续自动检测变更

4. **管理 MCP**（可选）
   - 「MCP 管理」→「添加 MCP」
   - 填写名称、命令、参数、环境变量
   - 勾选要应用的目标（Claude Code / Codex / Pi / Prime）
   - 保存后自动写入各目标的 MCP 配置

---

## 📂 数据目录

```
~/.xiaobai-switch/
├── xiaobai-switch.db   # SQLite 应用状态（加密 API Key、MCP 配置等）
├── master.key          # AES-256-GCM 主密钥（Unix 权限 0600）
└── backups/            # 应用前自动备份
    ├── agent-rules/    # 全局约束备份
    ├── mcp/            # MCP 配置备份
    └── xiaobai-switch-backup-*.tar.gz  # 完整数据库备份
```

**自动迁移**：从旧版 AnySwitch 升级时，首次启动会检测 `~/.any-switch/`，按数据库修改时间判断，复制较新的那份并校验。旧目录原样保留，不会删除。

**环境变量**：
- `XIAOBAI_SWITCH_DATA_DIR`：覆盖数据目录（兼容 `ANY_SWITCH_DATA_DIR`）
- `CLAUDE_CONFIG_DIR`：Claude Code 配置目录（默认 `~/.claude`）
- `CODEX_HOME`：Codex 配置目录（默认 `~/.codex`）
- `PI_CODING_AGENT_DIR`：Pi Agent 配置目录（默认 `~/.pi/agent`）
- `PRIME_AGENT_CODING_AGENT_DIR`：Prime Agent 配置目录（默认 `~/.prime/agent`）

---

## 🎯 使用场景

### 多站点切换
- 公司内网部署、自建中转、官方 API 多个站点并存
- 一键切换默认模型，无需手动编辑配置文件

### 多设备同步
- 家里台式机配置好站点、模型、MCP
- 公司笔记本打开应用，WebDAV 自动拉取，立即可用

### 团队协作
- 运维统一配置站点预设，生成 `xiaobaiswitchplus://` 深链
- 团队成员点击链接一键导入，无需手动抄写 URL 与 Key

### MCP 批量管理
- 一份 `@modelcontextprotocol/server-filesystem` 配置
- 勾选 Claude Code、Codex、Pi、Prime 四个目标
- 一次保存，四处生效

---

## 🛠️ 开发

### 环境要求
- Node.js 18+
- pnpm 8+
- Rust 1.75+
- Tauri CLI 2.x

### 本地运行

```bash
# 安装依赖
pnpm install

# 开发模式（热重载）
pnpm tauri dev

# 类型检查
pnpm typecheck

# 前端测试
pnpm test:run

# Rust 测试
cd src-tauri && cargo test

# 打包
pnpm tauri build
```

### 技术栈
- **前端**：React 18 + TypeScript + Vite + Ant Design 5 + Tailwind CSS v4
- **后端**：Rust + Tauri 2 + SQLite + rusqlite
- **状态管理**：zustand
- **国际化**：react-i18next（中文 + 英文）
- **图标**：Lucide React + @lobehub/icons

---

## 📖 文档

- **AGENTS.md**：Agent 与自动化约束（开发规范）
- **CHANGELOG.md**：版本变更记录
- **.zcode/tasks/**：Trellis 任务归档（设计文档、测试用例）

---

## ❓ 常见问题

### API Key 安全吗？
- 数据库内加密存储（AES-256-GCM）
- 应用到目标后，会以明文出现在 `~/.claude/settings.json`、`~/.codex/auth.json` 等文件中（这是各 CLI 的原生行为）
- 主密钥 `master.key` 权限 `0600`，仅当前用户可读

### WebDAV 同步会覆盖本地数据吗？
- 不会静默覆盖
- 首次同步：上传本地到远程
- 后续同步：比较指纹版本，自动检测哪边更新
- 冲突时：明确提示，由用户选择保留本地或拉取远程

### MCP 会干扰我手工配置的吗？
- 不会，只管理 `xiaobai_<name>` 前缀的条目
- 用户自己的 MCP（无前缀）完全不动
- 可选择将已有 MCP 纳管（扫描 → 导入）

### 如何恢复官方配置？
- 「应用中心」→ 对应目标 → 「恢复官方配置」
- 会删除所有 `xiaobai_*` 托管条目，保留用户自己的配置

### 支持哪些 CLI？
- **Claude Code**：写入 `~/.claude.json`（不是 `~/.claude/settings.json`）
- **Codex**：写入 `~/.codex/config.toml`、`~/.codex/models/xiaobai-model-catalog.json`
- **Pi**：写入 `~/.pi/agent/{models,auth,settings,mcp}.json`、`AGENTS.md`
- **Prime**：写入 `~/.prime/agent/{models,auth,settings}.json`、`AGENTS.md`

---

## 🤝 贡献

欢迎提 Issue 与 Pull Request！

请确保：
- 前端代码通过 `pnpm typecheck` 与 `pnpm test:run`
- Rust 代码通过 `cargo test` 与 `cargo clippy`
- 遵循 AGENTS.md 中的开发约束

---

## 📜 许可

MIT License

**原始项目**：[Licoy/xiaobai-switch](https://github.com/Licoy/xiaobai-switch)  
**增强版本**：本仓库在原项目基础上新增功能，保持 MIT 协议

---

## 🙏 致谢

- 感谢 [Licoy](https://github.com/Licoy) 创建原始项目
- 感谢所有贡献者的反馈与建议
