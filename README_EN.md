<p align="left">
  <a href="./README.md">中文</a> · <strong>English</strong>
</p>

<p align="center">
  <img src="assets/brand/app-icon-1024.png" alt="XiaoBaiSwitch Plus" width="160" height="160">
</p>

<h1 align="center">XiaoBaiSwitch Plus</h1>

<p align="center">
  <strong>All-in-One AI Coding Assistant Configuration Manager</strong><br>
  Unified management for Claude Code, Codex, Pi, Prime sites, models & configs
</p>

<p align="center">
  <a href="https://github.com/doMySelfZy/xiaobai-switch-plus/releases"><img src="https://img.shields.io/github/v/release/doMySelfZy/xiaobai-switch-plus?style=flat-square" alt="Release"></a>
  <a href="https://github.com/doMySelfZy/xiaobai-switch-plus/blob/main/LICENSE"><img src="https://img.shields.io/github/license/doMySelfZy/xiaobai-switch-plus?style=flat-square" alt="License"></a>
</p>

---

> **This project is an enhanced fork of [XiaoBaiSwitch](https://github.com/Licoy/xiaobai-switch) by [Licoy](https://github.com/Licoy).**  
> Original authorship, project, and MIT license belong to the original author. This repository adds enterprise-grade features including real WebDAV sync, unified MCP management, and global constraint injection, and publishes downloads and auto-updates from our own GitHub.

---

## ✨ Core Features

### 🎯 Site-First, Unified Management
- **One Site, Multiple Models**: Configure Base URL + API Key once per site, auto-fetch all available models
- **Target-Specific Configuration**: Claude Code, Codex, Pi, Prime each has dedicated forms, independent of each other
- **One-Click Apply**: Select targets and automatically write to each CLI's native config files, preserving format and comments

### 🔄 Real WebDAV Sync
- **Multi-Device Collaboration**: Laptop and desktop share one config set, sync on change, pull on switch
- **Smart Conflict Handling**: Auto-detect fingerprint versions, identify which side (local/remote) is newer, explicit prompt on conflict
- **Safe & Reliable**: Auto-backup before sync, verify database + master key together, never silently overwrite

### 🔌 Unified MCP Management
- **One Definition, Multiple Targets**: Edit MCP config once, select targets to write to Claude Code, Codex, Pi, Prime separately
- **Managed Namespace**: Only manages `xiaobai_*` prefixed entries, doesn't interfere with user's manual MCP configs
- **Scan & Import**: Can import existing manual MCP configs into management, secrets never pass through frontend

### 📝 Global Constraint Injection
- **Unified Agent Instructions**: Write Markdown constraints once, selectively inject into each target's `AGENTS.md` / `CLAUDE.md`
- **Managed Block Mechanism**: Use markers to delimit managed content, preserve user content outside blocks byte-for-byte
- **Auto-Cleanup**: When unchecking targets or clearing content, automatically remove managed blocks from corresponding targets

### 🔐 Security & Backup
- **Encrypted Storage**: API Keys and MCP secrets encrypted in database with AES-256-GCM, master key with `0600` permissions
- **Pre-Apply Backup**: Auto-backup to `~/.xiaobai-switch/backups/` before writing target configs
- **Restore Official Config**: One-click cleanup of managed entries, restore CLI to original state

### 🔗 Deep Link Import
- Supports `xiaobaiswitchplus://` protocol for one-click site preset import
- Compatible with legacy `anyswitch://` and `xiaobaiswitch://` share links

---

## 📥 Download & Update

- **GitHub Releases**: <https://github.com/doMySelfZy/xiaobai-switch-plus/releases>
- **Auto-Update**: In-app detection of `latest.json`, incremental patch download
- **No Website, No Gitee**

---

## 🚀 Quick Start

### Installation

1. Download platform-specific installer from [Releases](https://github.com/doMySelfZy/xiaobai-switch-plus/releases)
2. Windows: Run `.exe` installer; macOS: Drag to Applications; Linux: Follow distro instructions
3. First launch creates data directory at `~/.xiaobai-switch/` (auto-migrates from legacy `~/.any-switch/` if found)

### Basic Usage

1. **Add Site**
   - Click "Sites" → "Add Site"
   - Fill in name, Base URL, API Key
   - Click "Detect Protocol" to verify connectivity
   - Save to auto-fetch model list

2. **Configure Target**
   - Enter "App Center"
   - Select target (Claude Code / Codex / Pi / Prime)
   - Check sites and models to use
   - Configure default model, alias mappings, etc.
   - Click "Apply" to write target config

3. **Enable WebDAV Sync** (Optional)
   - "Settings" → "Data Sync"
   - Fill in WebDAV server address, username, password
   - Check "Enable Auto Sync"
   - First sync uploads local data, subsequent syncs auto-detect changes

4. **Manage MCP** (Optional)
   - "MCP Management" → "Add MCP"
   - Fill in name, command, args, environment variables
   - Check targets to apply (Claude Code / Codex / Pi / Prime)
   - Save to auto-write to each target's MCP config

---

## 📂 Data Directory

```
~/.xiaobai-switch/
├── xiaobai-switch.db   # SQLite app state (encrypted API Keys, MCP configs, etc.)
├── master.key          # AES-256-GCM master key (Unix permissions 0600)
└── backups/            # Auto-backups before apply
    ├── agent-rules/    # Global constraint backups
    ├── mcp/            # MCP config backups
    └── xiaobai-switch-backup-*.tar.gz  # Full database backups
```

**Auto-Migration**: When upgrading from legacy AnySwitch, first launch detects `~/.any-switch/`, compares database modification times, copies the newer one and verifies. Old directory preserved as-is, never deleted.

**Environment Variables**:
- `XIAOBAI_SWITCH_DATA_DIR`: Override data directory (compatible with `ANY_SWITCH_DATA_DIR`)
- `CLAUDE_CONFIG_DIR`: Claude Code config directory (default `~/.claude`)
- `CODEX_HOME`: Codex config directory (default `~/.codex`)
- `PI_CODING_AGENT_DIR`: Pi Agent config directory (default `~/.pi/agent`)
- `PRIME_AGENT_CODING_AGENT_DIR`: Prime Agent config directory (default `~/.prime/agent`)

---

## 🎯 Use Cases

### Multi-Site Switching
- Corporate intranet deployment, self-hosted proxy, official API coexist
- One-click switch default model, no manual config file editing

### Multi-Device Sync
- Configure sites, models, MCP on home desktop
- Open app on work laptop, WebDAV auto-pulls, ready to use

### Team Collaboration
- Ops configure site presets, generate `xiaobaiswitchplus://` deep links
- Team members click link to import, no manual URL/Key copying

### Batch MCP Management
- One `@modelcontextprotocol/server-filesystem` config
- Check Claude Code, Codex, Pi, Prime targets
- Save once, applies everywhere

---

## 🛠️ Development

### Requirements
- Node.js 18+
- pnpm 8+
- Rust 1.75+
- Tauri CLI 2.x

### Local Development

```bash
# Install dependencies
pnpm install

# Dev mode (hot reload)
pnpm tauri dev

# Type check
pnpm typecheck

# Frontend tests
pnpm test:run

# Rust tests
cd src-tauri && cargo test

# Build
pnpm tauri build
```

### Tech Stack
- **Frontend**: React 18 + TypeScript + Vite + Ant Design 5 + Tailwind CSS v4
- **Backend**: Rust + Tauri 2 + SQLite + rusqlite
- **State Management**: zustand
- **i18n**: react-i18next (Chinese + English)
- **Icons**: Lucide React + @lobehub/icons

---

## 📖 Documentation

- **AGENTS.md**: Agent and automation constraints (development guidelines)
- **CHANGELOG.md**: Version change log
- **.zcode/tasks/**: Trellis task archives (design docs, test cases)

---

## ❓ FAQ

### Are API Keys secure?
- Encrypted at rest in database (AES-256-GCM)
- After applying to targets, appear in plaintext in `~/.claude/settings.json`, `~/.codex/auth.json`, etc. (native CLI behavior)
- Master key `master.key` has `0600` permissions, readable only by current user

### Will WebDAV sync overwrite local data?
- Never silently overwrites
- First sync: Upload local to remote
- Subsequent syncs: Compare fingerprint versions, auto-detect which side is newer
- On conflict: Explicit prompt, user chooses to keep local or pull remote

### Will MCP interfere with my manual configs?
- No, only manages `xiaobai_<name>` prefixed entries
- User's own MCP (no prefix) completely untouched
- Can choose to import existing MCP (scan → import)

### How to restore official config?
- "App Center" → Corresponding target → "Restore Official Config"
- Deletes all `xiaobai_*` managed entries, preserves user's own configs

### Which CLIs are supported?
- **Claude Code**: Writes to `~/.claude.json` (not `~/.claude/settings.json`)
- **Codex**: Writes to `~/.codex/config.toml`, `~/.codex/models/xiaobai-model-catalog.json`
- **Pi**: Writes to `~/.pi/agent/{models,auth,settings,mcp}.json`, `AGENTS.md`
- **Prime**: Writes to `~/.prime/agent/{models,auth,settings}.json`, `AGENTS.md`

---

## 🤝 Contributing

Issues and Pull Requests welcome!

Please ensure:
- Frontend code passes `pnpm typecheck` and `pnpm test:run`
- Rust code passes `cargo test` and `cargo clippy`
- Follow development constraints in AGENTS.md

---

## 📜 License

MIT License

**Original Project**: [Licoy/xiaobai-switch](https://github.com/Licoy/xiaobai-switch)  
**Enhanced Version**: This repository adds features on top of the original project, maintains MIT license

---

## 🙏 Acknowledgments

- Thanks to [Licoy](https://github.com/Licoy) for creating the original project
- Thanks to all contributors for feedback and suggestions
