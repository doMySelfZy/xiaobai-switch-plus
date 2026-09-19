# MCP 纳管与同步的身份去重

## 背景

本机库实测 7 条 MCP，其中 `sequential-thinking` / `sequentialthinking` 与 `ssh` / `ssh-mcp-server`
是同一个服务器被各存了两遍。解密比对后的证据：

| 对比 | 归一化 command | args | 差异 |
|---|---|---|---|
| `sequential-thinking` vs `sequentialthinking` | `npx` = `npx` | 完全相同 | 后者多带 Claude 私有字段 `enabled`/`timeoutMs`/`type` |
| `ssh` vs `ssh-mcp-server` | `npx` = `npx` | **不同** | 一条多 `--whitelist`，密码也不同 |

根因是**身份模型只有「名字」一个字段，而名字是每个客户端各自的局部属性**：

- 纳管时 `name` 直接取客户端配置里的键名（Claude 侧写 `sequentialthinking`，Codex 侧写
  `sequential-thinking`），且 `id` 恒为新建；
- `repo::mcp::save` 的重名校验**只比名字**，两个不同名字畅通无阻；
- 扫描结果标记「已纳管」的 `imported_id` **也只按名字**匹配。

同步把这个缺陷放大：WebDAV 是**整库替换**（不做行级 merge）。换库后进来的行名字与本机客户端
键名对不上，那条手工条目就又显示成「未纳管」→ 再纳管一次 → 两个名字被同步永久固化在共享库里，
两台机器互相贡献。

下游后果：托管键由 `xiaobai_<name>` 派生，所以只要勾选目标开始应用，同一个 MCP 会以**两条**
`xiaobai_*` 写进同一个客户端，客户端加载两份同样的工具集；既有的「同名未托管条目接管」比对也救不了，
因为它按同名匹配。

同根因带出的第二个缺陷：Claude 的私有字段（`type`/`enabled`/`timeoutMs`）随 `config` 原样透传，
会被写进 Codex 的 TOML（Codex 不认这些键）。另外 `mcp_version.rs` 判 `command == "npx"` 是字面
比较，绝对路径 `npx.cmd` 的条目连版本检测都认不出来。

## 需求

### 身份口径

1. 引入两层身份，都从既有数据在读取时计算，**不新增数据库列**：
   - **粗身份**（判「是不是同一个服务器」）：归一化 command + args + url + 传输类型；
   - **等价**（判「能不能合并」）：粗身份 + `env`/`headers` 的值。
2. command 归一化规则：取 basename、剥掉 `.cmd`/`.exe`/`.bat`、大小写不敏感。
   `npx`、`npx.cmd`、`D:\Program Files\nodejs\npx.cmd` 必须归一到同一个值。
3. 比对时忽略、但**存储时原样保留**的客户端私有字段：`type`、`transport`、`enabled`、`timeoutMs`。
   绝不改写用户已入库的 `config`。

### 纳管与保存

4. 纳管命中已有行的粗身份 → **只标记不建行**：扫描列表把该条显示为「已纳管为 <库内名字>」，
   不新建行，也不修改已有行（不改名、不改 `targets`、不刷新 `updated_at`）。
5. 手工新增/编辑保存时命中**另一行**的粗身份 → 明确报错并指出冲突条目的名字，不得静默写入。
   按 `id` 编辑自身不受影响。
6. 最后一道防线：应用时若同一目标上有两条启用的服务共享粗身份，**不得把两条都写进客户端**，
   该目标报明确失败并说明是哪两条。

### 存量重复项

7. 打开 MCP 面板时检测重复分组，给横幅提示「发现 N 组重复」，列出组内每条的名字、来源与绑定的目标。
8. 组内**等价** → 提供一键合并，默认保留条的选取规则固定且可预测：优先有目标绑定的 → 其次启用的 →
   最后创建时间最早的。
9. 组内**不等价**（如本机 `ssh` 那对）→ 展示并排差异，**必须由用户显式指定保留哪一条**才执行合并，
   界面不预选。
10. 合并动作必须先做整库快照备份；被丢弃条目在各客户端里的 `xiaobai_*` 必须随后被清理掉。
11. 任何情况下不得删除用户客户端文件里的手工条目（沿用既有「不等价就报错并原样保留」红线）。

### 同步

12. 换库后（下载并重启落地）重复检测必须照常生效；**启动时绝不自动合并**，只检测并提示。
13. 不得改动 `mcp_servers` 表结构与 `FINGERPRINT_TABLES` 清单——改表结构会改变逻辑指纹，
    触发跨设备互相覆盖的风险（`sync.rs` 里有钉死断言）。

### 附带修复

14. 版本检测的命令识别复用同一套归一化，使绝对路径 `npx.cmd` / `uvx` 条目也能被认出。

### 兼容红线

15. 托管键仍是 `xiaobai_<name>`，`name` 的语义不变；深链 scheme、备份前缀、数据目录、
    `xiaobai_` 命名空间等既有协议值一律不动。

## 验收标准

- [ ] 单测：`npx` 与 `D:\Program Files\nodejs\npx.cmd` + 相同 args → 粗身份相同；args 不同 → 粗身份不同。
- [ ] 单测：Claude 形状（带 `type`/`timeoutMs`/`enabled`）与 Codex 形状的同一条目 → 等价成立。
- [ ] 单测：纳管命中粗身份时不产生新行，且已有行的 name / targets / updated_at 均未变。
- [ ] 单测：`save` 新增行命中他行粗身份 → 报错且错误信息含冲突条目名字；按 id 编辑自身仍通过。
- [ ] 单测：应用时同目标两条共享粗身份 → 该目标失败，两条都不写入。
- [ ] 单测：合并等价组 → 只剩一行、`targets` 为并集、被丢弃行的 `xiaobai_<name>` 从目标客户端消失。
- [ ] 单测：合并不等价组未显式指定保留 id → 拒绝执行，库与客户端文件零改动。
- [ ] 单测：合并前生成 `pre_mcp_merge` 快照；合并中途失败时不删任何行。
- [ ] `cargo test` 全绿；`pnpm typecheck` + `pnpm test:run` 全绿。
- [ ] 真机：本机现有 7 行里正确报出 2 组重复，`sequential-thinking` 组判「等价」、`ssh` 组判「非等价」。
- [ ] 真机：勾选目标并应用一次后，`~/.codex/config.toml` 里同一服务器只有一条 `xiaobai_*`。
- [ ] 回归：`FINGERPRINT_TABLES` 与 `FINGERPRINT_ALGORITHM_VERSION` 未变，`sync.rs` 的钉死断言仍通过。

## 范围外（另案）

- `config_json` 明文存放 args 里的密码：本机 `ssh` 条目实测把密码写在 `--password` 之后，
  库里不加密、`~/.codex/config.toml` 里也是明文。属既有「`config` 原样透传」设计，本任务不动，
  但须在交付说明里提醒用户把密码挪到 `env`（那一列走 AES-GCM）。
