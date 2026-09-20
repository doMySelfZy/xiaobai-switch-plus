# Backend Development Guidelines

> 本项目的实际后端约定。**目标是可直接照着写代码**，不是原则宣讲。

---

## 权威源与本文档的关系

`AGENTS.md`（仓库根）是**约束的唯一权威源**——产品规则、兼容红线、目录布局、测试要求
都在那里，改动必须以它为准。

本目录是按层提炼的**操作性索引**：改同步引擎时看
[webdav-sync](./webdav-sync.md)，要移除一个已经落库的枚举值/应用目标时看
[target-retirement](./target-retirement.md)。两处冲突时以 `AGENTS.md` 为准。

---

## Guidelines Index

| Guide | Description | Status |
|-------|-------------|--------|
| [WebDAV Sync](./webdav-sync.md) | 同步引擎的四条不变量、manifest 契约、启动恢复协议 | ✅ |
| [Target Retirement](./target-retirement.md) | 删除已落库的枚举值/应用目标：清洗先于删枚举、两种拼写、列留值空、失败矩阵与守门测试 | ✅ |

---

## 快速上手（最常踩的四条）

1. **兼容红线只做"新旧双识别"，绝不改协议值**：manifest 文件名、备份前缀
   （`xiaobai-switch-backup-` / `any-switch-backup-`）、备份包内条目名
   `xiaobai-switch.db`、`xiaobai_` 命名空间。改这些会让既有数据不可读。
2. **改 `FINGERPRINT_TABLES` 必须递增 `FINGERPRINT_ALGORITHM_VERSION`**：表清单是指纹算法
   的一部分，不递增会让跨版本的两台机器互相覆盖（真实发生过，见 webdav-sync.md 不变量 3）。
3. **schema 版本号变更时，`apply_schema` 的每个「库已存在」分支都要跑
   `ensure_incremental_schema`**：`CREATE TABLE IF NOT EXISTS` 不会给已有表补列/补表，
   漏掉分支会让老库拿不到新表新列**且不报错**。
4. **删掉一个 `TargetKind` 变体之前先写清洗层**：`TargetKind` 没有 `#[serde(other)]`，
   残留值会让整份文档反序列化失败；且 serde 的 `snake_case` 把 `ZCode` 落成 `"z_code"`
   而 `as_str()` 落成 `"zcode"`，两种拼写在存量里都存在。全流程见
   [target-retirement](./target-retirement.md)。

---

## 提交前机械检查（实测口径，别按"CI 一定会拦"想当然）

`.github/workflows/` 里**没有** `cargo fmt` / `cargo clippy` 步骤，仓库也没有 `rustfmt.toml`
（根与 `src-tauri` 均无）→ 这两项都不是本仓门禁，也不存在"全仓 clean"这个基线。实测：
`cargo clippy --offline --lib --tests` 有 ~122 条既有警告；被改的 15 个 `.rs` 里 10 个不合
rustfmt 默认，仅 `lib.rs` 就有 795 行 rustfmt 想动，而它单次任务只新增 13 行。

- **必跑**：`src-tauri` 下 `cargo test`（前端另见 `frontend/quality-guidelines.md`）。
- **clippy 只用增量口径**：`cargo clippy --offline --lib --tests --message-format=short`，
  把警告的 `file:line` 与本次 `git diff -U0` 的新增行区间求交集，**只修交集里的**。
  禁止 `cargo clippy -- -D warnings`（会在无关既有码上红，逼你顺手重构）。
- **禁止 `cargo fmt`**：没有配置文件时它是 rustfmt 默认风格，与本仓既有风格冲突，
  跑一次产出的巨量无关 diff 会淹掉真实改动。新文件也不必单独"洗白"成 rustfmt 默认——
  那会让一个文件与仓库其余部分风格分裂。
- **增量核查脚本必须自证解析到了东西**：`rustfmt --check` 打的是它自己的
  `Diff in <path>:<line>:` 头，**不是** `@@ -a,b +c,d @@`。按 unified diff 去解析会一行都
  匹配不上，从而报告"0 处需要改"的**假阴性**——比不查更危险，因为它给出通过的样子。
  这类脚本要打印中间量（"解析到 N 个文件的新增行"），确认 N 非零再信结论。

**Language**: 文档用中文写；代码标识符、路径、命令保持原文。

---

## 本地 NSIS 构建验证（实测口径，2026-09-20）

- 命令：`pnpm tauri build --bundles nsis`（仅 NSIS；`tauri.conf.json` 默认
  `targets: all` 会连打 MSI，耗时翻倍）。
- **Gotcha：无 `TAURI_SIGNING_PRIVATE_KEY` 时构建尾部 updater 签名报 exit 1，
  但 NSIS `.exe` 已产出**——先看 `src-tauri/target/release/bundle/nsis/` 有无产物
  再判失败，不要只看退出码（updater 签名链路与安装包验证无关）。
- 开打前确认版本三处一致：`package.json` / `src-tauri/tauri.conf.json` /
  `src-tauri/Cargo.toml`（本次 0.1.6）。
- 覆盖安装前先备份注册表 `InstallLocation` 与数据目录实际解析路径
  （`XIAOBAI_SWITCH_DATA_DIR` → `ANY_SWITCH_DATA_DIR` → `~/.xiaobai-switch`）；
  NSIS `/S` 静默安装会沿用注册表旧路径，不会产生第二份安装。
- 启动验证口径：进程存活 + `MainWindowHandle` 非零 + `Responding` +
  站点库只读可查；无截图手段时如实声明图像断言缺失，不虚报。