# 诊断证据（2026-09-19 本机实测）

本文档是规划阶段的调查结论，供实现与检查阶段直接引用。所有比对均**只打印等值结果**，
未导出任何密钥明文；下文出现的 `<password>` 处为真实凭据，已脱敏。

## 库内现状

`~/.xiaobai-switch/xiaobai-switch.db` 的 `mcp_servers` 共 7 行，`targets_json` **全为 `[]`**
（即从未应用过），`sync_meta` 里也没有 `mcp_applied_targets`：

```
context7              id=d8d352e7  command=C:\Users\...\npm\context7-mcp.cmd
exa                   id=28ae1aeb  url=https://mcp.exa.ai/mcp?tools=...
sequential-thinking   id=dc89ec64  command=npx   args=[-y, @modelcontextprotocol/server-sequential-thinking]
sequentialthinking    id=caa79cf4  command=D:\Program Files\nodejs\npx.cmd   args=同上（完全相同）
ssh                   id=40be2369  command=D:\Program Files\nodejs\npx.cmd   args=[-y, @fangjunjie/ssh-mcp-server,
                                             --host, nas.tenbase.cn, --port, 10005, --username, zhangyang,
                                             --password, <password>]
ssh-mcp-server        id=e67d77a3  command=npx   args=[..., --password, <password>, --whitelist, <len=134>]
tavily                id=2e5ad2dd  command=npx   args=[-y, tavily-mcp]
```

本机客户端文件现状：

- `~/.codex/config.toml` 的 `[mcp_servers.*]`：`context7` / `exa` / `sequential-thinking` / `ssh` / `tavily`
  （其中 `exa` `ssh` `tavily` 带 `.env` 子表）
- `~/.claude.json` 顶层 `mcpServers`：**空**
- Pi / Prime 配置文件不存在

## 两组重复的等值比对（解密后比对，未打印值）

| 对比 | 归一化 command | args | env/headers 值 | config 键集差异 |
|---|---|---|---|---|
| `sequential-thinking` vs `sequentialthinking` | `npx` = `npx` ✅ | 相同 ✅ | 相等（都空）✅ | 后者多 `enabled` / `timeoutMs` / `type` |
| `ssh` vs `ssh-mcp-server` | `npx` = `npx` ✅ | **不同** ❌ | 相等（都空）✅ | 前者多 `timeoutMs` / `type` |

推论：带 `type`/`enabled`/`timeoutMs` 的行来自 **Claude Code**，不带的来自 **Codex**。
Claude 侧的手工条目现已消失（`mcpServers` 为空），库里那两行成了指向不存在来源的孤儿行。

**关键结论：`ssh` 那对不是「同一条存两次」，而是同包同主机的两套不同配法**（一条带 `--whitelist`、
密码也不同）。因此「按内容等价自动合并」会漏掉它，且**绝不能**静默合并 —— 必须让用户点选保留条。
这直接决定了身份要分两层（粗身份 / 等价）。

## 根因链（代码位置）

1. `adapters/mcp_scan.rs:444`（Codex 分支）与 `:473`（JSON 分支）——
   导入时 `name = normalize_scanned_name(key)` 取客户端自己的键名，`id: None` 恒新建。
2. `repo/mcp.rs:158-170` —— 唯一性校验 `WHERE name = ?1 COLLATE NOCASE AND id <> ?2`，**只比名字**。
3. `commands/mcp.rs:384-391` —— 扫描结果的 `imported_id` 用 `by_name`（名字小写）映射，**也只按名字**。
4. `sync.rs` 的 `decide_action` + Upload/Download —— 同步是**整库替换**，不做行级 merge。
   换库后进来的行名字与本机客户端键名对不上 → 那条手工条目又显示「未纳管」→ 再纳管 → 多一行。
   两台机器各贡献一个名字，重复被同步永久固化。
5. 下游：`adapters/mcp.rs:33` `managed_key(name)` → 一旦勾选目标，同一个 MCP 会以
   `xiaobai_sequential-thinking` + `xiaobai_sequentialthinking` **两条**写进同一客户端。
   既有接管比对（`adapters/mcp.rs:93` `untracked_matches_record`）按**同名**未托管条目匹配，救不了。

## 同根因带出的两个附带缺陷

- **客户端私有字段污染共享记录**：Claude 的 `type` / `enabled` / `timeoutMs` 随 `config` 原样透传，
  应用时会被写进 Codex 的 TOML（Codex 不认这些键）。`adapters/mcp.rs:75` 的 `strip_entry` 目前
  只在等价比对时忽略 `type` / `transport`，未覆盖 `enabled` / `timeoutMs`。
- **版本检测认不出绝对路径**：`adapters/mcp_version.rs:22` 判 `command == "npx"` 是字面比较，
  `D:\Program Files\nodejs\npx.cmd` 这类条目直接跳过。

## 边界事实（影响设计选型）

- `repo/*` 对 `crate::adapters` **零依赖**（已 grep 核实）→ 去重守卫放命令层，不下沉到 repo。
- `repo::mcp::save` 的调用方只有两处：`commands/mcp.rs:165`（手工保存）与 `:440`（纳管）。
- `mcp_servers` 在 `FINGERPRINT_TABLES`（`sync.rs:41-51`）里，且指纹走 `SELECT *` →
  **新增列会改变逻辑指纹**，需递增 `FINGERPRINT_ALGORITHM_VERSION` 并两端同时升级。
  `sync.rs:717-735` 有钉死断言。故身份一律读取时计算，不落库。
- `mcp_applied_targets` 存在 `sync_meta`（不参与指纹、不跨机同步）→ 语义正确（描述本机文件），
  但换库后本机陈旧的 `xiaobai_*` 要等下一次 apply 才被扫掉。
- 启动换库落点：`state.rs:22` `apply_pending_restore`。
- 前端：纳管是内联 `Card`（`McpPage.tsx:1008-1133`）不是 Modal；`importable` 过滤在 `:609-612`；
  主表 `Table` 在 `:1140-1147`，空 targets 渲染 `—`（`:777`）。
  可复用的横幅形状是 `ProxyPage.tsx:340-345`（`RulesPage.tsx:173-189` 用了 antd Alert 不认的 `title=`，
  **不要抄**）。i18n 是单一扁平 bundle：`src/i18n/locales/{zh-CN,en-US}.json`，MCP 键前缀 `mcp.`。
  `browserMock.ts` 未知命令会 throw（`:2223-2227`），新命令必须补 `case`。
