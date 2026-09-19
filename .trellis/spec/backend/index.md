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

**Language**: 文档用中文写；代码标识符、路径、命令保持原文。
