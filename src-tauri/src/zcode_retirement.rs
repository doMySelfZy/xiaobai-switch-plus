//! ZCode 目标撤退清理器（U1 数据撤退层）。
//!
//! ## 为什么存在
//!
//! 产品要移除 ZCode 这个应用目标，但「删代码」与「删既成事实」是两件事：
//!
//! 1. `TargetKind` 被序列化进了多处**持久化** JSON（`settings` blob、`mcp_servers.targets_json`、
//!    `agent_rules.targets_json`、`sync_meta` 的两份 applied 目标集合）。serde 没开
//!    `#[serde(other)]`，删掉枚举变体会让**整份**值反序列化失败 —— 先删枚举，得到的是一个
//!    打不开老用户数据库的构建。
//! 2. 本应用已经把 `xiaobai_*` 条目与托管块写进了用户机器上的 `~/.zcode/`。删掉写入器
//!    同时也删掉了唯一的清理器，那些条目会变成永久孤儿（`AGENTS.md` 的孤儿清理红线）。
//! 3. 逻辑指纹对 `sites` 走 `SELECT *` 逐列哈希，所以 `sites.zcode_api_type` 只能**置空值**、
//!    不能删列 —— 删列会改变所有既有库的指纹，重演 0.1.3 / 0.1.4 的跨机互相覆盖事故。
//!
//! 因此本模块必须在删枚举**之前**落地，并且必须能在枚举**还存在**时独立工作（幂等清洗）。
//!
//! ## 什么条件下可以整文件删除
//!
//! 以下三条同时成立时，本文件连同 `lib.rs` setup 里的 `clean_external_once` 调用点、
//! `db/migrate.rs::ensure_incremental_schema` 里的挂载点一起删除：
//!
//! 1. v0.1.5 及更早版本已不再被任何在用机器打开（升级窗口结束）；
//! 2. 对端不会再推含 zcode 的旧数据（跨机混跑窗口结束，见 design.md §6）；
//! 3. 产品明确接受用户机器 `~/.zcode` 里可能残留的 `xiaobai_*` 条目，或已确认清理完毕。
//!
//! ## 自包含约束（勿破坏）
//!
//! 本模块只认识字符串：`"zcode"` / `"z_code"` 字面量、`xiaobai_` 前缀、托管块标记、
//! `~/.zcode` 下三处落点的相对路径。**不 import `adapters::zcode`**（U3 删），
//! **不使用 `TargetKind::ZCode`**（U2 删）。落盘路径常量也只在本模块内定义，
//! 避免 `paths.rs` 把 ZCode 专用路径继续暴露给业务层。
//!
//! ## 已知副作用（设计接受，见 design.md §6）
//!
//! 清洗发生在 `Db::open` → `ensure_incremental_schema` 里，必然改动本地数据、进而改变逻辑
//! 指纹。v0.1.5 用户升级后第一次同步会被判成「只有本地变了」→ **Upload**，把撤退后的数据发布
//! 出去，对端随后 Download。这是预期行为：不要在这里加例外、不要改记账时机、不要把清洗挪到
//! 同步之后。

use crate::adapters::atomic::{atomic_write, backup_file, FileLock};
use crate::db::{column_exists, table_exists};
use crate::error::{AppError, AppResult};
use crate::repo::sync_meta;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{Map, Value};
use std::fs;
use std::path::{Path, PathBuf};

// ============================================================
// 字面量：整个后端只在这里认识 ZCode
// ============================================================

/// JSON 数组元素里出现过的目标名。**两种写法都要认**：
/// - `"z_code"` 是 `TargetKind::ZCode` 经 `#[serde(rename_all = "snake_case")]` 真正落库的形式；
/// - `"zcode"` 是 `TargetKind::as_str()` 的形式（design.md §3.1 的清洗清单按它写），也是任何
///   手工修过库、或将来某条路径误写时会出现的形式。
const RETIRED_JSON_NAMES: [&str; 2] = ["z_code", "zcode"];

/// `target_bindings.target` / `apply_records.target` 列里的目标名（写入方是 `as_str()`）。
const RETIRED_COLUMN_VALUES: [&str; 2] = ["zcode", "z_code"];

/// 本应用写进 ZCode 配置里的托管条目命名空间（兼容红线：该前缀永不改）。
const MANAGED_PREFIX: &str = "xiaobai_";

/// 全局约束的托管块标记（与 `adapters/agent_rules.rs` 同源；此处刻意不 import：
/// 撤退脚本不该依赖一个还在演进的写路径）。
const RULE_BEGIN: &str = "<!-- xiaobai-switch:begin global-rules -->";
const RULE_END: &str = "<!-- xiaobai-switch:end global-rules -->";
const BOM: &str = "\u{feff}";

/// `~/.zcode` 下的三处落点（相对路径）。
const PATH_PROVIDER: &str = "v2/config.json";
const PATH_PROVIDER_RULES: &str = "v2/provider_config.json";
const PATH_MCP: &str = "cli/config.json";
const MARKDOWN_TARGETS: [&str; 2] = ["AGENTS.md", "CLAUDE.md"];

/// 记着「哪些目标写过托管内容」的两份账本，必须一起瘦身：留着退役名会让整个数组
/// 解析失败，进而把**所有**目标的清理集合静默清零（design.md §3.3）。
const APPLIED_TARGET_KEYS: [&str; 2] = ["agent_rules_applied_targets", "mcp_applied_targets"];

/// v0.1.5 的 `AppSettings` 里那个「自定义 ZCode 目录」字段序列化后的键名。结构体字段已删，
/// 但 `settings` blob 在被第一次重新保存之前仍带着它 —— 文件侧清理靠它找回自定义目录。
const LEGACY_OVERRIDE_KEY: &str = "zcodeHomeOverride";

/// 清洗前快照与文件备份的落点子目录名。两处基目录不同：
/// 整库快照走 `app_backups_dir()` → `~/.xiaobai-switch/backups/app/zcode-retirement/`，
/// 被改写的用户配置文件走 `backups_dir()` → `~/.xiaobai-switch/backups/zcode-retirement/`。
const DB_SNAPSHOT_DIR: &str = "zcode-retirement";

/// 日志里只出现文件名，绝不出现内容（可能含 API key）或完整路径（含用户名）。
fn label_of(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("unknown")
        .to_string()
}

// ============================================================
// 数据库侧：ensure_in_db
// ============================================================

/// 一次清洗的结果。`touched` 只描述**已落库**的变更，`warnings` 描述被跳过的可疑值。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DbChanges {
    pub touched: Vec<String>,
    pub warnings: Vec<String>,
}

impl DbChanges {
    pub fn is_empty(&self) -> bool {
        self.touched.is_empty()
    }
}

/// 主键在两张表里的实际类型不同（`mcp_servers.id` 是 TEXT，`agent_rules.id` 是
/// `INTEGER PRIMARY KEY`），所以按 `rusqlite::types::Value` 原样读出、原样绑回 UPDATE，
/// 不做字符串 coercion —— `row.get::<_, String>` 读到整型主键会直接报
/// `Invalid column type Integer`，那会让整次清洗失败。
type SqlValue = rusqlite::types::Value;

/// 只用于日志与 `touched` 描述，不回绑 SQL。
fn describe_pk(value: &SqlValue) -> String {
    match value {
        SqlValue::Integer(i) => i.to_string(),
        SqlValue::Text(s) => s.clone(),
        SqlValue::Null => "NULL".to_string(),
        SqlValue::Real(f) => f.to_string(),
        SqlValue::Blob(_) => "<blob>".to_string(),
    }
}

/// 一条待改写的 JSON 单元格。表名/列名全部来自本文件的字面量，绝不来自数据。
struct CellEdit {
    table: &'static str,
    column: &'static str,
    pk_column: &'static str,
    pk_value: SqlValue,
    new_json: String,
    label: String,
}

#[derive(Default)]
struct Plan {
    settings_json: Option<String>,
    cells: Vec<CellEdit>,
    meta: Vec<(&'static str, String)>,
    binding_rows: i64,
    apply_record_rows: i64,
    clear_zcode_api_type: bool,
    warnings: Vec<String>,
}

impl Plan {
    fn is_empty(&self) -> bool {
        self.settings_json.is_none()
            && self.cells.is_empty()
            && self.meta.is_empty()
            && self.binding_rows == 0
            && self.apply_record_rows == 0
            && !self.clear_zcode_api_type
    }
}

/// 生产入口：幂等地把库里指向 ZCode 的数据外科手术掉。
///
/// 由 `db/migrate.rs::ensure_incremental_schema` 调用，所以首启、升级、备份恢复、外部库校验
/// 四条路径都会跑到。失败不得冒泡成「数据库打不开」，调用点负责降级为 warn。
pub(crate) fn ensure_in_db(conn: &Connection) -> AppResult<DbChanges> {
    ensure_in_db_with(conn, backup_before_first_write)
}

/// 把「写前整库备份」抽成参数，让「只备份一次 / no-op 不备份」可被单测覆盖，
/// 且单测不会往真实 `~/.xiaobai-switch/backups/` 写东西。
fn ensure_in_db_with(
    conn: &Connection,
    backup: impl FnOnce(&Connection) -> AppResult<()>,
) -> AppResult<DbChanges> {
    let plan = build_plan(conn)?;
    for warning in &plan.warnings {
        tracing::warn!(warning = %warning, "ZCode 撤退清洗跳过一处可疑值");
    }
    if plan.is_empty() {
        // 干净库：一次写都不做，指纹不动，也不留备份痕迹。
        return Ok(DbChanges {
            touched: Vec::new(),
            warnings: plan.warnings,
        });
    }
    // 有损删除（DELETE / 置 NULL）之前必须先有一份完整快照。
    backup(conn)?;
    apply_plan(conn, &plan)
}

fn build_plan(conn: &Connection) -> AppResult<Plan> {
    let mut plan = Plan::default();
    if table_exists(conn, "settings")? {
        plan_settings(conn, &mut plan)?;
    }
    if table_exists(conn, "mcp_servers")? {
        plan_json_column(conn, "mcp_servers", "targets_json", "id", &mut plan)?;
    }
    if table_exists(conn, "agent_rules")? {
        plan_json_column(conn, "agent_rules", "targets_json", "id", &mut plan)?;
    }
    if table_exists(conn, "sync_meta")? {
        plan_applied_targets(conn, &mut plan)?;
    }
    if table_exists(conn, "target_bindings")? {
        plan.binding_rows = count_retired_rows(conn, "target_bindings")?;
    }
    if table_exists(conn, "apply_records")? {
        plan.apply_record_rows = count_retired_rows(conn, "apply_records")?;
    }
    if table_exists(conn, "sites")? && column_exists(conn, "sites", "zcode_api_type")? {
        plan.clear_zcode_api_type = count_zcode_api_type_rows(conn)? > 0;
    }
    Ok(plan)
}

/// 预筛用的带引号字面量：`zcodeHomeOverride` 这类键名不该被误判成脏数据，
/// 自由文本里提到 zcode 更不该触发一次写盘。
fn contains_retired_name(text: &str) -> bool {
    RETIRED_JSON_NAMES
        .iter()
        .any(|name| text.contains(&format!("\"{name}\"")))
}

fn is_retired_name(value: &str) -> bool {
    RETIRED_JSON_NAMES.iter().any(|name| *name == value)
}

/// 从 JSON 里剔除退役目标的字符串数组元素，返回是否真的改动了。
///
/// 走 `Value` 往返而不是手工切字符串（脆弱性换健壮性）。代价：`serde_json` 未开
/// `preserve_order`，`Map` 即 `BTreeMap`，被改写的那一行键序会重排成字母序 ——
/// design.md §3.1 明确接受（下一次正常保存会按结构体声明序重写）。
fn strip_retired(value: &mut Value) -> bool {
    match value {
        Value::Array(items) => {
            let before = items.len();
            items.retain(|item| !matches!(item, Value::String(text) if is_retired_name(text)));
            let mut changed = items.len() != before;
            for item in items.iter_mut() {
                changed |= strip_retired(item);
            }
            changed
        }
        Value::Object(entries) => {
            let mut changed = false;
            for (_key, nested) in entries.iter_mut() {
                changed |= strip_retired(nested);
            }
            changed
        }
        _ => false,
    }
}

/// `settings` 表 `id = 1` 的整份 AppSettings blob。
///
/// 不认具体键名（实际落库的是驼峰 `localProxyTargets`，design.md §3.1 写的是
/// `local_proxy_targets`），改认「任一层字符串数组里的退役目标名」。
fn plan_settings(conn: &Connection, plan: &mut Plan) -> AppResult<()> {
    let Some(raw) = conn
        .query_row("SELECT json FROM settings WHERE id = 1", [], |row| {
            row.get::<_, String>(0)
        })
        .optional()?
    else {
        return Ok(());
    };
    if !contains_retired_name(&raw) {
        return Ok(());
    }
    let mut value: Value = match serde_json::from_str(&raw) {
        Ok(value) => value,
        Err(error) => {
            plan.warnings
                .push(format!("settings(id=1) 无法解析，原样保留：{error}"));
            return Ok(());
        }
    };
    if !strip_retired(&mut value) {
        return Ok(());
    }
    plan.settings_json = Some(serde_json::to_string(&value)?);
    Ok(())
}

/// `mcp_servers.targets_json` 与 `agent_rules.targets_json`：逐行处理，只改脏的那一行。
fn plan_json_column(
    conn: &Connection,
    table: &'static str,
    column: &'static str,
    pk_column: &'static str,
    plan: &mut Plan,
) -> AppResult<()> {
    let sql = format!("SELECT {pk_column}, {column} FROM {table}");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, SqlValue>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut edits = Vec::new();
    for row in rows {
        let (pk_value, raw) = row?;
        if !contains_retired_name(&raw) {
            continue;
        }
        let pk_label = describe_pk(&pk_value);
        let mut value: Value = match serde_json::from_str(&raw) {
            Ok(value) => value,
            Err(error) => {
                plan.warnings.push(format!(
                    "{table}.{column}(pk={pk_label}) 无法解析，原样保留：{error}"
                ));
                continue;
            }
        };
        if !strip_retired(&mut value) {
            continue;
        }
        edits.push(CellEdit {
            table,
            column,
            pk_column,
            label: format!("{table}.{column}(pk={pk_label})"),
            new_json: serde_json::to_string(&value)?,
            pk_value,
        });
    }
    drop(stmt);
    plan.cells.extend(edits);
    Ok(())
}

/// `sync_meta` 里两份「已应用目标」账本。
fn plan_applied_targets(conn: &Connection, plan: &mut Plan) -> AppResult<()> {
    let mut edits = Vec::new();
    for key in APPLIED_TARGET_KEYS {
        let Some(raw) = sync_meta::get_meta(conn, key)? else {
            continue;
        };
        if !contains_retired_name(&raw) {
            continue;
        }
        let mut value: Value = match serde_json::from_str(&raw) {
            Ok(value) => value,
            Err(error) => {
                plan.warnings
                    .push(format!("sync_meta({key}) 无法解析，原样保留：{error}"));
                continue;
            }
        };
        if !strip_retired(&mut value) {
            continue;
        }
        edits.push((key, serde_json::to_string(&value)?));
    }
    plan.meta.extend(edits);
    Ok(())
}

fn count_retired_rows(conn: &Connection, table: &'static str) -> AppResult<i64> {
    let sql = format!("SELECT COUNT(*) FROM {table} WHERE target = ?1 OR target = ?2");
    let count: i64 = conn.query_row(
        &sql,
        params![RETIRED_COLUMN_VALUES[0], RETIRED_COLUMN_VALUES[1]],
        |row| row.get(0),
    )?;
    Ok(count)
}

fn count_zcode_api_type_rows(conn: &Connection) -> AppResult<i64> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sites WHERE zcode_api_type IS NOT NULL AND zcode_api_type <> ''",
        [],
        |row| row.get(0),
    )?;
    Ok(count)
}

fn apply_plan(conn: &Connection, plan: &Plan) -> AppResult<DbChanges> {
    let mut changes = DbChanges {
        touched: Vec::new(),
        warnings: plan.warnings.clone(),
    };

    if let Some(json) = &plan.settings_json {
        conn.execute("UPDATE settings SET json = ?1 WHERE id = 1", params![json])?;
        changes.touched.push("settings".to_string());
    }

    for edit in &plan.cells {
        let sql = format!(
            "UPDATE {} SET {} = ?1 WHERE {} = ?2",
            edit.table, edit.column, edit.pk_column
        );
        conn.execute(&sql, params![edit.new_json, edit.pk_value])?;
        changes.touched.push(edit.label.clone());
    }

    for (key, json) in &plan.meta {
        sync_meta::set_meta(conn, key, json)?;
        changes.touched.push(format!("sync_meta({key})"));
    }

    if plan.binding_rows > 0 {
        let deleted = delete_retired_rows(conn, "target_bindings")?;
        changes.touched.push(format!("target_bindings({deleted} 行)"));
    }
    if plan.apply_record_rows > 0 {
        let deleted = delete_retired_rows(conn, "apply_records")?;
        changes.touched.push(format!("apply_records({deleted} 行)"));
    }

    if plan.clear_zcode_api_type {
        // 只置空值，绝不 DROP COLUMN：列集合就是逻辑指纹的一部分。
        let updated = conn.execute(
            "UPDATE sites SET zcode_api_type = NULL WHERE zcode_api_type IS NOT NULL AND zcode_api_type <> ''",
            [],
        )?;
        changes.touched.push(format!("sites.zcode_api_type({updated} 行)"));
    }

    Ok(changes)
}

fn delete_retired_rows(conn: &Connection, table: &'static str) -> AppResult<usize> {
    let sql = format!("DELETE FROM {table} WHERE target = ?1 OR target = ?2");
    let deleted = conn.execute(
        &sql,
        params![RETIRED_COLUMN_VALUES[0], RETIRED_COLUMN_VALUES[1]],
    )?;
    Ok(deleted)
}

/// 真实备份：整库快照到应用备份目录下本模块专用的一格。
///
/// 复用 `db::migrate::backup_snapshot`（`backup_pre_migration` 的同一条路径），但有三处刻意差别：
/// 1. 独立子目录，绝不覆盖用户既有的 `pre_migration_site_api_keys` 快照；
/// 2. `replace_existing = false` —— 快照已存在就不重写，要留的是**清洗前**那一份，不是最近一份；
/// 3. `require_master_key = false` —— 缺 `master.key` 只 warn，不报错：清洗推迟到下次启动无所谓，
///    让 `Db::open` 失败会让用户连应用都打不开。
/// 另：内存库/临时连接（备份校验等）不做快照，与 `maybe_backup(Auto)` 同口径。
fn backup_before_first_write(conn: &Connection) -> AppResult<()> {
    let path = conn.path().map(Path::new);
    let is_file_db = path.is_some_and(|p| p.exists() && p.file_name().is_some());
    if !is_file_db {
        tracing::debug!("ZCode 撤退：非落盘数据库，跳过整库快照");
        return Ok(());
    }
    crate::db::migrate::backup_snapshot(conn, DB_SNAPSHOT_DIR, false, false)?;
    Ok(())
}

// ============================================================
// 文件侧：clean_external_once
// ============================================================

/// 一次文件撤退的结果。`written` = 真的改了盘的文件；`skipped` = 形状不认识、原样保留并
/// warn 的文件。两者皆空即 no-op。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FileReport {
    pub written: Vec<PathBuf>,
    pub skipped: Vec<PathBuf>,
}

impl FileReport {
    pub fn is_no_op(&self) -> bool {
        self.written.is_empty() && self.skipped.is_empty()
    }
}

/// 退役前那一版把用户自定义的 ZCode 目录记在 `settings` blob 的 `zcodeHomeOverride` 键里。
/// 结构体字段已随目标一起删（U3），但 `ensure_in_db` 只剔数组元素、不碰这个键，所以升级后
/// **第一次**启动仍能从盘上读到它——晚一步就会被第一次 `save_settings` 冲掉。
/// 读不到（或本就空白）只清默认根目录。
pub(crate) fn leftover_home_override(conn: &Connection) -> AppResult<Option<String>> {
    let raw: Option<String> = conn
        .query_row("SELECT json FROM settings WHERE id = 1", [], |row| row.get(0))
        .optional()?;
    let Some(raw) = raw else {
        return Ok(None);
    };
    let Ok(value) = serde_json::from_str::<Value>(&raw) else {
        // blob 已经坏到读不出键名：不是这里的职责（`repo::settings` 会报），静默降级。
        return Ok(None);
    };
    Ok(value
        .get(LEGACY_OVERRIDE_KEY)
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty()))
}

/// 启动时调用一次（`lib.rs` setup）。每次启动都跑，靠幂等保证安全；刻意**不加**
/// `sync_meta` 完成标记 —— 标记会被同步带到别的机器，反而让那台机器的 `~/.zcode`
/// 永远清不掉（design.md §5）。
///
/// `leftover_override` 取自 [`leftover_home_override`]：配过自定义目录的用户，我们把
/// `xiaobai_*` 条目（含明文 key）写进了那个目录，因此那个目录也在清理范围内。
///
/// 失败一律内部降级为 warn：这是清理，不是启动前置条件。
pub(crate) fn clean_external_once(leftover_override: Option<&str>) -> FileReport {
    let mut roots: Vec<PathBuf> = Vec::new();
    match zcode_root() {
        Ok(root) => merge_root(&mut roots, root),
        Err(error) => {
            tracing::warn!(error = %error, "ZCode 撤退：默认目标目录解析失败");
        }
    }
    if let Some(path) = leftover_override.map(str::trim).filter(|s| !s.is_empty()) {
        merge_root(&mut roots, PathBuf::from(path));
    }
    if roots.is_empty() {
        return FileReport::default();
    }
    let backup_root = match crate::paths::backups_dir() {
        Ok(dir) => dir.join(DB_SNAPSHOT_DIR),
        Err(error) => {
            tracing::warn!(error = %error, "ZCode 撤退：备份目录不可用，跳过文件清理");
            return FileReport::default();
        }
    };
    let mut report = FileReport::default();
    for root in roots {
        let one = clean_external_in(&root, &backup_root);
        report.written.extend(one.written);
        report.skipped.extend(one.skipped);
    }
    if !report.is_no_op() {
        tracing::info!(
            written = report.written.len(),
            skipped = report.skipped.len(),
            "ZCode 撤退：外部配置清理完成"
        );
    }
    report
}

/// 同一个目录可能被解析成两种写法（`D:\zcode` 与 `D:\zcode\.`）；清两遍虽然幂等，
/// 但第二遍会白取一次锁，所以先按 `canonicalize()` 归一，拿不到就按字面比。
fn merge_root(roots: &mut Vec<PathBuf>, candidate: PathBuf) {
    let key = candidate.canonicalize().unwrap_or_else(|_| candidate.clone());
    if roots
        .iter()
        .any(|root| root.canonicalize().unwrap_or_else(|_| root.clone()) == key)
    {
        return;
    }
    roots.push(candidate);
}

/// 根路径解析，与被删除的 `paths::default_zcode_home` 同一口径：`ZCODE_HOME` 优先、
/// 空白视作未设置，最后回落到 `~/.zcode`。override 设置项随 ZCode 目标一起退役了，
/// 结构体里不再有它，所以这里是默认根目录的唯一解析入口。
fn zcode_root() -> AppResult<PathBuf> {
    if let Ok(from_env) = std::env::var("ZCODE_HOME") {
        let trimmed = from_env.trim();
        if !trimmed.is_empty() {
            return Ok(PathBuf::from(trimmed));
        }
    }
    Ok(crate::paths::home_dir()?.join(".zcode"))
}

/// 实际清理。`root` / `backup_root` 全部来自参数，单测因此只碰 tempdir。
fn clean_external_in(root: &Path, backup_root: &Path) -> FileReport {
    let mut report = FileReport::default();
    if !root.exists() {
        // 用户根本没装 ZCode：全程静默，不建目录、不 warn。
        return report;
    }
    prune_managed_map_file(
        root.join(PATH_PROVIDER),
        &["provider"],
        backup_root,
        &mut report,
    );
    prune_provider_rules(root.join(PATH_PROVIDER_RULES), backup_root, &mut report);
    prune_managed_map_file(
        root.join(PATH_MCP),
        &["mcp", "servers"],
        backup_root,
        &mut report,
    );
    strip_managed_blocks(root, backup_root, &mut report);
    report
}

/// 一个 JSON 落点的处理结论。
enum Shape {
    /// 这里没有我们的东西。
    Nothing,
    /// 改动了，值得写盘。
    Changed,
    /// 形状不认识，整份原样保留。
    Unknown(&'static str),
}

/// 托管条目表（`v2/config.json` 的 `provider`、`cli/config.json` 的 `mcp.servers`）：
/// 键是条目名，托管条目带 `xiaobai_` 前缀。
fn prune_managed_map_file(
    path: PathBuf,
    keys: &'static [&'static str],
    backup_root: &Path,
    report: &mut FileReport,
) {
    prune_json_file(&path, backup_root, true, report, move |root| {
        match dig_map(root, keys) {
            Dig::Absent => Shape::Nothing,
            Dig::Malformed => Shape::Unknown("托管容器不是 JSON 对象"),
            Dig::Node(map) => {
                if strip_managed_keys(map) {
                    Shape::Changed
                } else {
                    Shape::Nothing
                }
            }
        }
    });
}

/// `v2/provider_config.json`：`providerOrder` 与两张规则表都按 providerId 指认托管条目。
fn prune_provider_rules(path: PathBuf, backup_root: &Path, report: &mut FileReport) {
    prune_json_file(&path, backup_root, true, report, |root| {
        let config = match dig_map(root, &["config"]) {
            Dig::Node(map) => map,
            Dig::Absent => return Shape::Nothing,
            Dig::Malformed => return Shape::Unknown("config 不是 JSON 对象"),
        };
        let mut changed = false;

        match config.get_mut("providerOrder") {
            None => {}
            Some(Value::Array(items)) => {
                let before = items.len();
                items.retain(|item| {
                    !matches!(item, Value::String(text) if text.starts_with(MANAGED_PREFIX))
                });
                changed |= items.len() != before;
            }
            Some(_) => return Shape::Unknown("config.providerOrder 不是数组"),
        }

        for (group_key, rules_field) in [
            ("providerConfigRules", "providerRules"),
            ("modelConfigRules", "providerModelRules"),
        ] {
            let Some(group) = config.get_mut(group_key) else {
                continue;
            };
            let Value::Object(group) = group else {
                return Shape::Unknown("规则分组不是 JSON 对象");
            };
            let Some(rules) = group.get_mut(rules_field) else {
                continue;
            };
            let Value::Array(items) = rules else {
                return Shape::Unknown("规则列表不是数组");
            };
            let before = items.len();
            items.retain(|item| !is_managed_provider_entry(item));
            changed |= items.len() != before;
        }

        if changed {
            Shape::Changed
        } else {
            Shape::Nothing
        }
    });
}

fn is_managed_provider_entry(item: &Value) -> bool {
    let Some(object) = item.as_object() else {
        return false;
    };
    matches!(object.get("providerId"), Some(Value::String(id)) if id.starts_with(MANAGED_PREFIX))
}

fn strip_managed_keys(map: &mut Map<String, Value>) -> bool {
    let before = map.len();
    map.retain(|key, _value| !key.starts_with(MANAGED_PREFIX));
    map.len() != before
}

enum Dig<'a> {
    /// 路径上某层的键根本不存在 —— 这份文件里没有我们的东西，静默。
    Absent,
    /// 键存在但类型不对 —— 不能猜着改。
    Malformed,
    /// 拿到了可变的对象表。
    Node(&'a mut Map<String, Value>),
}

/// 递归下钻（迭代写法会在 `node = next` 上撞借用检查器）。
fn dig_map<'a>(node: &'a mut Value, keys: &[&'static str]) -> Dig<'a> {
    let Some((head, rest)) = keys.split_first() else {
        return match node.as_object_mut() {
            Some(map) => Dig::Node(map),
            None => Dig::Malformed,
        };
    };
    match node.get_mut(*head) {
        None => Dig::Absent,
        Some(child) => dig_map(child, rest),
    }
}

fn prune_json_file(
    path: &Path,
    backup_root: &Path,
    secret: bool,
    report: &mut FileReport,
    prune: impl FnOnce(&mut Value) -> Shape,
) {
    if !path.exists() {
        // 静默：该落点压根不存在（不创建、不 warn）。
        return;
    }
    let Some(guard) = lock_or_skip(path, report) else {
        return;
    };
    let Some(mut root) = read_json_existing(path, report) else {
        return;
    };
    match prune(&mut root) {
        Shape::Nothing => {}
        Shape::Changed => match write_json_existing(path, &root, secret, backup_root) {
            Ok(()) => report.written.push(path.to_path_buf()),
            Err(error) => skip(path, report, &format!("写回失败: {error}")),
        },
        Shape::Unknown(reason) => skip(path, report, reason),
    }
    drop(guard);
}

fn read_json_existing(path: &Path, report: &mut FileReport) -> Option<Value> {
    if path.is_dir() {
        skip(path, report, "目标不是文件");
        return None;
    }
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) => {
            skip(path, report, &format!("读取失败: {error}"));
            return None;
        }
    };
    let text = match String::from_utf8(bytes) {
        Ok(text) => text,
        Err(_) => {
            skip(path, report, "不是 UTF-8");
            return None;
        }
    };
    match serde_json::from_str(&text) {
        Ok(value) => Some(value),
        // serde 的报错会带上 JSON 片段（可能含密钥），所以只说「不合法」，不回报道文。
        Err(_) => {
            skip(path, report, "不是合法 JSON");
            None
        }
    }
}

fn write_json_existing(
    path: &Path,
    root: &Value,
    secret: bool,
    backup_root: &Path,
) -> AppResult<()> {
    backup_file(path, backup_root)?;
    let text = serde_json::to_string_pretty(root)? + "\n";
    atomic_write(path, text.as_bytes(), secret)
}

fn strip_managed_blocks(root: &Path, backup_root: &Path, report: &mut FileReport) {
    for name in MARKDOWN_TARGETS {
        let path = root.join(name);
        if !path.exists() {
            continue;
        }
        let Some(guard) = lock_or_skip(&path, report) else {
            continue;
        };
        let text = match read_text_existing(&path) {
            Ok(text) => text,
            Err(error) => {
                skip(&path, report, &error.to_string());
                continue;
            }
        };
        let had_bom = text.starts_with(BOM);
        let body = text.strip_prefix(BOM).unwrap_or(&text);
        match strip_block(body) {
            // 文件里本来就没有托管块：不写、不备份。
            Ok(None) => {}
            Ok(Some(next)) => {
                if let Err(error) = remove_or_write_markdown(&path, &next, had_bom, backup_root) {
                    skip(&path, report, &format!("写回失败: {error}"));
                } else {
                    report.written.push(path.to_path_buf());
                }
            }
            Err(reason) => skip(&path, report, reason),
        }
        drop(guard);
    }
}

fn read_text_existing(path: &Path) -> AppResult<String> {
    if path.is_dir() {
        return Err(AppError::new("invalid_config", "目标不是文件"));
    }
    let bytes = fs::read(path)?;
    String::from_utf8(bytes).map_err(|_| AppError::new("invalid_config", "不是 UTF-8"))
}

/// `Ok(None)` = 没有托管块（不写、不备份）；`Ok(Some)` = 块外剩余内容。
fn strip_block(text: &str) -> Result<Option<String>, &'static str> {
    if !text.contains(RULE_BEGIN) {
        return Ok(None);
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(begin_at) = rest.find(RULE_BEGIN) {
        out.push_str(&rest[..begin_at]);
        let after_begin = &rest[begin_at..];
        let Some(end_rel) = after_begin.find(RULE_END) else {
            // 文件被手工改坏：不能猜着改，原样保留。
            return Err("托管块只有开始标记没有结束标记");
        };
        rest = &after_begin[end_rel + RULE_END.len()..];
    }
    out.push_str(rest);
    Ok(Some(tidy_remainder(&out)))
}

/// 与写路径同一口径：块外内容逐字节保留，唯一允许的字节变化是尾部空行归一。
fn tidy_remainder(text: &str) -> String {
    let trimmed = text.trim_end_matches(['\n', '\r']);
    if trimmed.trim().is_empty() {
        return String::new();
    }
    format!("{trimmed}\n")
}

fn remove_or_write_markdown(
    path: &Path,
    next: &str,
    had_bom: bool,
    backup_root: &Path,
) -> AppResult<()> {
    backup_file(path, backup_root)?;
    if next.trim().is_empty() {
        // 整份文件都是本应用写的：删掉。留一个空 AGENTS.md 会反过来遮蔽用户自己的 CLAUDE.md。
        fs::remove_file(path)?;
    } else {
        let payload = if had_bom {
            format!("{BOM}{next}")
        } else {
            next.to_string()
        };
        atomic_write(path, payload.as_bytes(), false)?;
    }
    Ok(())
}

fn lock_or_skip(path: &Path, report: &mut FileReport) -> Option<FileLock> {
    match FileLock::acquire(path) {
        Ok(guard) => Some(guard),
        Err(error) => {
            skip(path, report, &format!("拿不到互斥锁: {error}"));
            None
        }
    }
}

fn skip(path: &Path, report: &mut FileReport, reason: &str) {
    tracing::warn!(
        file = %label_of(path),
        reason,
        "ZCode 撤退：跳过该文件，内容原样保留"
    );
    report.skipped.push(path.to_path_buf());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{AppSettings, TargetKind};
    use crate::repo;
    use serde_json::json;

    const MASTER: [u8; 32] = [7u8; 32];

    fn crypto() -> crate::crypto::Crypto {
        crate::crypto::Crypto::from_key(MASTER)
    }

    fn fingerprint(conn: &Connection) -> String {
        crate::sync::compute_logical_fingerprint(conn, &MASTER)
            .unwrap()
            .database_sha256
    }

    fn apply_clean(conn: &Connection) -> DbChanges {
        super::ensure_in_db_with(conn, |_| Ok(())).unwrap()
    }

    /// 记录备份钩子被调了几次、以及**每次调用时库里还有没有退役名**。
    /// 后半截是为了证明「先整库备份、再写盘」的顺序：把 `backup(conn)?` 挪到
    /// `apply_plan` 之后，`still_dirty` 就会变成 `false`，这里的断言立刻变红。
    #[derive(Default)]
    struct BackupProbe {
        calls: usize,
        still_dirty: Vec<bool>,
    }

    fn apply_clean_with_probe(conn: &Connection, probe: &mut BackupProbe) -> DbChanges {
        let sink = probe;
        super::ensure_in_db_with(conn, |conn| {
            sink.calls += 1;
            sink.still_dirty.push(
                conn.query_row("SELECT json FROM settings WHERE id = 1", [], |row| {
                    row.get::<_, String>(0)
                })
                .map(|raw| raw.contains("\"z_code\""))
                .unwrap_or(false),
            );
            Ok(())
        })
        .unwrap()
    }

    fn stored(conn: &Connection, sql: &str) -> String {
        conn.query_row(sql, [], |row| row.get::<_, String>(0))
            .unwrap()
    }

    fn count(conn: &Connection, sql: &str) -> i64 {
        conn.query_row(sql, [], |row| row.get::<_, i64>(0))
            .unwrap()
    }

    fn columns_of(conn: &Connection, table: &str) -> Vec<String> {
        let mut stmt = conn
            .prepare(&format!("PRAGMA table_info({table})"))
            .unwrap();
        stmt.query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    }

    /// v0.1.5 形状库。脏值一律用**裸字符串**写库，测试本身不引用 `TargetKind::ZCode` ——
    /// 这样 U2 删掉枚举变体之后，这批守门测试仍然编译、仍然在守同一批契约。
    fn schema_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::apply_schema(&conn).unwrap();
        // 真实应用第一次读设置就会落一行默认 blob，存量库必然有这一行。
        repo::settings::get_settings(&conn).unwrap();
        conn
    }

    fn seed_v15(conn: &Connection) {
        let mut settings = serde_json::to_value(AppSettings::default()).unwrap();
        settings.as_object_mut().unwrap().insert(
            "localProxyTargets".to_string(),
            json!(["claude_code", "z_code", "pi"]),
        );
        // v0.1.5 的 `AppSettings` 里有「自定义 ZCode 目录」；字段已随目标删除，
        // 但序列化出来的 blob 在被重新保存之前仍带着这个键。
        settings.as_object_mut().unwrap().insert(
            LEGACY_OVERRIDE_KEY.to_string(),
            json!("D:/custom/zcode"),
        );
        conn.execute(
            "INSERT INTO settings (id, json) VALUES (1, ?1)
             ON CONFLICT(id) DO UPDATE SET json = excluded.json",
            params![settings.to_string()],
        )
        .unwrap();

        conn.execute(
            "INSERT INTO sites (id, name, base_url, protocol, claude_auth_key_style, enabled,
                                sort_order, created_at, updated_at, zcode_api_type)
             VALUES ('site-1', '站点一', 'https://a.example.com', 'anthropic',
                     'anthropic_auth_token', 1, 0, 1, 1, 'anthropic-messages')",
            [],
        )
        .unwrap();

        conn.execute(
            "INSERT INTO mcp_servers (id, name, kind, enabled, targets_json, config_json,
                                       secrets_encrypted, created_at, updated_at)
             VALUES ('mcp-1', 'demo', 'stdio', 1, '[\"claude_code\",\"z_code\"]',
                     '{\"command\":\"mcp-demo\"}', NULL, 1, 1)",
            [],
        )
        .unwrap();

        conn.execute(
            "INSERT INTO agent_rules (id, body, targets_json, updated_at)
             VALUES (1, '说中文', '[\"claude_code\",\"z_code\"]', 1)",
            [],
        )
        .unwrap();

        conn.execute(
            "INSERT INTO sync_meta (key, value, updated_at)
             VALUES ('agent_rules_applied_targets', '[\"claude_code\",\"codex\",\"z_code\"]', 1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sync_meta (key, value, updated_at)
             VALUES ('mcp_applied_targets', '[\"z_code\",\"pi\"]', 1)",
            [],
        )
        .unwrap();

        conn.execute(
            "INSERT INTO target_bindings (target, site_id, site_name_snapshot, model_id, provider_id,
                                          key_fingerprint, managed_paths_json, managed_env_keys_json,
                                          expected_fields_json, orphan, apply_record_id, applied_at)
             VALUES ('zcode', 'site-1', '站点一', 'claude-opus', 'xiaobai_site1', 'fp-z', '[]', '[]', '{}',
                     0, NULL, 1),
                    ('claude_code', 'site-1', '站点一', 'claude-opus', NULL, 'fp-c', '[]', '[]', '{}',
                     0, NULL, 1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO apply_records (id, site_id, site_name_snapshot, target, model_id, provider_id,
                                        status, backup_dir, touched_keys_json, config_snapshot_hash,
                                        error, applied_at)
             VALUES ('rec-z', 'site-1', '站点一', 'zcode', 'claude-opus', NULL, 'ok', NULL, '[]', NULL,
                     NULL, 1),
                    ('rec-p', 'site-1', '站点一', 'pi', 'claude-opus', NULL, 'ok', NULL, '[]', NULL,
                     NULL, 1)",
            [],
        )
        .unwrap();
    }

    fn v15_conn() -> Connection {
        let conn = schema_conn();
        seed_v15(&conn);
        conn
    }

    // ============================================================
    // AC1：存量不炸 —— 设置可读可存
    // ============================================================

    #[test]
    fn settings_blob_keeps_readable_and_writable_after_retirement() {
        let conn = v15_conn();
        assert!(
            stored(&conn, "SELECT json FROM settings WHERE id = 1").contains("\"z_code\""),
            "夹具必须真的把退役目标塞进 settings blob，否则这条断言没有意义"
        );

        let changes = apply_clean(&conn);
        assert!(
            !changes.is_empty(),
            "脏库上撤退清洗必须真的写到东西：{changes:?}"
        );

        let raw = stored(&conn, "SELECT json FROM settings WHERE id = 1");
        // 只认**带引号的数组元素**：键名 `zcodeHomeOverride` 与它的值都不算脏数据 ——
        // 文件侧清理正靠这个键定位用户自定义的 ZCode 目录（design.md §3.1 也没要求删它，
        // 删它反而多写一次盘，并且会让那批用户的明文 key 永久残留）。
        assert!(
            !raw.contains("\"z_code\"") && !raw.contains("\"zcode\""),
            "blob 里不应再有退役目标值：{raw}"
        );
        assert!(
            raw.contains("claude_code") && raw.contains("pi"),
            "其它接管目标必须原样保留：{raw}"
        );

        assert_eq!(
            repo::settings::get_settings(&conn)
                .unwrap()
                .local_proxy_targets,
            vec![TargetKind::ClaudeCode, TargetKind::Pi]
        );

        // 「可存」：走真实的 merge_settings（先 get_settings 再 save_settings）。
        let merged = repo::settings::merge_settings(&conn, json!({ "themeMode": "dark" })).unwrap();
        assert_eq!(merged.theme_mode, "dark");
        assert_eq!(
            repo::settings::get_settings(&conn)
                .unwrap()
                .local_proxy_targets,
            vec![TargetKind::ClaudeCode, TargetKind::Pi],
            "保存必须能落库，且不会把退役目标带回来"
        );
    }

    // ============================================================
    // AC1 / AC3：MCP 与全局约束可读，清理集合不被清零
    // ============================================================

    #[test]
    fn mcp_list_and_applied_targets_survive_the_retirement() {
        let conn = v15_conn();
        let key = crypto();
        // U2 删掉枚举变体之后，清洗**之前**读不动才是正确行为：`repo/mcp.rs:55` 走严格的
        // `from_str::<Vec<TargetKind>>`，design.md §5 明确选了「一次性写库清洗」而不是
        // 「读路径容错」。这条断言钉住由此产生的顺序要求 —— `ensure_in_db` 必须跑在任何
        // 读路径前面，容错一旦加回读路径它就该红。
        let before = repo::mcp::list(&conn, &key).unwrap_err().to_string();
        assert!(
            before.contains("z_code") || before.contains("zcode"),
            "清洗前必须因退役目标解析失败，实际错误：{before}"
        );

        apply_clean(&conn);

        let listed = repo::mcp::list(&conn, &key).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].targets, vec![TargetKind::ClaudeCode]);
        assert_eq!(
            stored(&conn, "SELECT targets_json FROM mcp_servers WHERE id='mcp-1'"),
            r#"["claude_code"]"#
        );

        // AC3：MCP 托管条目的清理目标集合必须还在（未知值会让它整体退化成 []）。
        assert_eq!(
            repo::mcp::applied_targets(&conn).unwrap(),
            vec![TargetKind::Pi],
            "mcp_applied_targets 里的退役目标必须被剔掉，其余目标不得被清零"
        );
    }

    #[test]
    fn agent_rules_targets_stay_readable_and_non_empty() {
        let conn = v15_conn();
        apply_clean(&conn);

        let rules = repo::rules::get(&conn).unwrap();
        assert_eq!(rules.body, "说中文");
        assert_eq!(rules.targets, vec![TargetKind::ClaudeCode]);

        assert_eq!(
            repo::rules::applied_targets(&conn).unwrap(),
            vec![TargetKind::ClaudeCode, TargetKind::Codex],
            "已应用目标集合只该掉掉退役目标，不得整体变空"
        );
    }

    // ============================================================
    // AC2 / design §3.3：残留行不伪装成 Claude Code
    // ============================================================

    #[test]
    fn zcode_binding_rows_are_dropped_and_never_take_over_another_target() {
        let conn = v15_conn();
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM target_bindings"), 2);

        apply_clean(&conn);

        assert_eq!(
            count(&conn, "SELECT COUNT(*) FROM target_bindings WHERE target='zcode'"),
            0
        );
        assert_eq!(
            count(&conn, "SELECT COUNT(*) FROM apply_records WHERE target='zcode'"),
            0
        );
        assert_eq!(
            count(&conn, "SELECT COUNT(*) FROM apply_records WHERE target='pi'"),
            1,
            "只有退役目标的行该消失"
        );

        let bindings = repo::binding::list_bindings(&conn).unwrap();
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0].target, TargetKind::ClaudeCode);
    }

    #[test]
    fn unknown_target_binding_is_skipped_instead_of_masquerading_as_claude_code() {
        let conn = schema_conn();
        conn.execute(
            "INSERT INTO target_bindings (target, site_id, site_name_snapshot, model_id, provider_id,
                                          key_fingerprint, managed_paths_json, managed_env_keys_json,
                                          expected_fields_json, orphan, apply_record_id, applied_at)
             VALUES ('totally_unknown', NULL, '幽灵', 'm', NULL, 'fp', '[]', '[]', '{}', 0, NULL, 1)",
            [],
        )
        .unwrap();

        apply_clean(&conn);

        assert_eq!(
            count(&conn, "SELECT COUNT(*) FROM target_bindings"),
            1,
            "清洗只删退役目标的行，未知目标行由读取层负责忽略"
        );
        assert!(
            repo::binding::list_bindings(&conn)
                .unwrap()
                .is_empty(),
            "未知目标不得被当成 Claude Code 回传（那会让下游用 claude 的清理路径去动用户真实配置）"
        );
        assert!(repo::binding::get_binding(&conn, TargetKind::ClaudeCode)
            .unwrap()
            .is_none());
    }

    // ============================================================
    // R2 / 兼容红线：列置空但不 DROP，SCHEMA_VERSION 与协议值不动
    // ============================================================

    #[test]
    fn site_zcode_api_type_is_cleared_without_touching_the_column_set() {
        let conn = v15_conn();
        let columns_before = columns_of(&conn, "sites");
        let version_before = crate::db::user_version(&conn).unwrap();

        apply_clean(&conn);

        let value: Option<String> = conn
            .query_row(
                "SELECT zcode_api_type FROM sites WHERE id='site-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(value, None, "值该置 NULL");
        assert_eq!(
            columns_of(&conn, "sites"),
            columns_before,
            "列集合必须逐位不变：改动它等于改逻辑指纹，会重演跨机互相覆盖事故"
        );
        assert!(columns_before.contains(&"zcode_api_type".to_string()));
        assert_eq!(
            crate::db::user_version(&conn).unwrap(),
            version_before,
            "清洗是数据外科手术，不得递增 schema 版本"
        );
        assert_eq!(crate::db::SCHEMA_VERSION, 3);
        assert_eq!(crate::sync::FINGERPRINT_ALGORITHM_VERSION, 1);
        assert_eq!(crate::sync::SYNC_FORMAT_VERSION, 1);
        assert_eq!(
            crate::sync::SYNC_MANIFEST_FILE_NAME,
            "xiaobai-switch-sync.json"
        );
    }

    // ============================================================
    // AC4：幂等 + 只在真有写操作时备份一次
    // ============================================================

    #[test]
    fn retirement_is_idempotent_and_backs_up_exactly_once() {
        let conn = v15_conn();
        let mut probe = BackupProbe::default();

        let first = apply_clean_with_probe(&conn, &mut probe);
        assert!(!first.is_empty(), "首轮必须报告变更：{first:?}");
        assert_eq!(probe.calls, 1, "有写操作时必须先备份整个库，且只备份一次");
        assert_eq!(
            probe.still_dirty,
            vec![true],
            "备份必须发生在写盘**之前**：快照里要的正是还没被清洗的那份数据"
        );

        let second = apply_clean_with_probe(&conn, &mut probe);
        assert!(second.is_empty(), "第二次必须是彻底的 no-op：{second:?}");
        assert_eq!(probe.calls, 1, "no-op 轮次不得再备份、再写盘");
    }

    #[test]
    fn clean_database_is_left_byte_identical_and_unbacked_up() {
        let conn = schema_conn();
        conn.execute(
            "INSERT INTO agent_rules (id, body, targets_json, updated_at) VALUES (1, 'x', '[]', 1)",
            [],
        )
        .unwrap();
        let before_settings = stored(&conn, "SELECT json FROM settings WHERE id = 1");
        let before_fp = fingerprint(&conn);
        let mut probe = BackupProbe::default();

        let changes = apply_clean_with_probe(&conn, &mut probe);

        assert!(changes.is_empty(), "干净库上不该有任何变更：{changes:?}");
        assert_eq!(probe.calls, 0, "没有写操作就不该备份");
        assert_eq!(
            stored(&conn, "SELECT json FROM settings WHERE id = 1"),
            before_settings,
            "settings blob 未经清洗也必须逐字节不变（serde 往返会把键重排成字母序）"
        );
        assert_eq!(
            fingerprint(&conn),
            before_fp,
            "指纹只该随真实业务值变化"
        );
    }

    #[test]
    fn dirty_database_retirement_changes_the_local_fingerprint_on_purpose() {
        // 已知且刻意接受的后果（design.md §6）：清洗改了本地数据 → 指纹变 →
        // 升级后第一次同步判成「只有本地变了」→ Upload 发布撤退后的数据。
        let conn = v15_conn();
        let before = fingerprint(&conn);
        apply_clean(&conn);
        assert_ne!(
            fingerprint(&conn),
            before,
            "撤退数据必须由本地发起发布，不得为同步判定开例外"
        );
    }

    #[test]
    fn malformed_json_is_left_alone_but_reported() {
        let conn = schema_conn();
        conn.execute(
            "INSERT INTO agent_rules (id, body, targets_json, updated_at) VALUES (1, 'x', '[\"claude_code\",\"z_code\"', 1)",
            [],
        )
        .unwrap();
        let before = stored(&conn, "SELECT targets_json FROM agent_rules WHERE id = 1");

        let changes = apply_clean(&conn);

        assert!(
            changes.touched.is_empty(),
            "坏 JSON 不是本次的修复入口：{changes:?}"
        );
        assert_eq!(changes.warnings.len(), 1, "坏值必须被记录：{changes:?}");
        assert_eq!(
            stored(&conn, "SELECT targets_json FROM agent_rules WHERE id = 1"),
            before,
            "无法解析的值必须原样保留"
        );
    }

    #[test]
    fn the_other_spelling_of_the_retired_target_is_cleaned_too() {
        let conn = schema_conn();
        conn.execute(
            "INSERT INTO agent_rules (id, body, targets_json, updated_at) VALUES (1, 'x', '[\"claude_code\",\"zcode\"]', 1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO target_bindings (target, site_id, site_name_snapshot, model_id, provider_id,
                                          key_fingerprint, managed_paths_json, managed_env_keys_json,
                                          expected_fields_json, orphan, apply_record_id, applied_at)
             VALUES ('z_code', NULL, '另一种写法', 'm', NULL, 'fp', '[]', '[]', '{}', 0, NULL, 1)",
            [],
        )
        .unwrap();

        apply_clean(&conn);

        assert_eq!(
            stored(&conn, "SELECT targets_json FROM agent_rules WHERE id = 1"),
            r#"["claude_code"]"#
        );
        assert_eq!(
            count(&conn, "SELECT COUNT(*) FROM target_bindings WHERE target='z_code'"),
            0
        );
    }

    #[test]
    fn free_text_mentioning_the_name_does_not_trigger_a_write() {
        let conn = schema_conn();
        let before_settings = stored(&conn, "SELECT json FROM settings WHERE id = 1");
        conn.execute(
            "INSERT INTO sync_meta (key, value, updated_at) VALUES ('probe', 'zcode 用户指南', 1)",
            [],
        )
        .unwrap();
        let before_meta = stored(&conn, "SELECT value FROM sync_meta WHERE key='probe'");
        let before_fp = fingerprint(&conn);
        let mut probe = BackupProbe::default();

        let changes = apply_clean_with_probe(&conn, &mut probe);

        assert!(
            changes.is_empty(),
            "自由文本里提到退役名不算脏数据：{changes:?}"
        );
        assert_eq!(probe.calls, 0, "无脏数据却拍了整库快照");
        assert_eq!(
            stored(&conn, "SELECT value FROM sync_meta WHERE key='probe'"),
            before_meta
        );
        assert_eq!(
            stored(&conn, "SELECT json FROM settings WHERE id = 1"),
            before_settings
        );
        assert_eq!(fingerprint(&conn), before_fp);
    }

    // ============================================================
    // 挂载点：apply_schema 的三个「库已存在」分支都要跑到
    // ============================================================

    #[test]
    fn mounted_on_the_up_to_date_branch_of_apply_schema() {
        let conn = v15_conn();
        assert_eq!(
            crate::db::user_version(&conn).unwrap(),
            crate::db::SCHEMA_VERSION
        );
        crate::db::apply_schema(&conn).unwrap();
        assert!(
            !stored(&conn, "SELECT json FROM settings WHERE id = 1").contains("z_code"),
            "版本号已达标的存量库也要被清洗（早退分支同样要挂）"
        );
    }

    #[test]
    fn mounted_on_the_legacy_migration_branch() {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::migrate::install_legacy_schema(&conn).unwrap();
        let crypto = crypto();
        conn.execute(
            "INSERT INTO sites (id, name, base_url, api_key_encrypted, key_prefix, protocol,
                                claude_auth_key_style, notes, enabled, sort_order, created_at, updated_at)
             VALUES ('site-1', '站点一', ?1, ?2, 'sk-…', 'anthropic', 'anthropic_auth_token',
                     NULL, 1, 0, 1, 1)",
            params!["https://a.example.com", crypto.encrypt("sk-legacy").unwrap()],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO target_bindings (target, site_id, site_name_snapshot, model_id, provider_id,
                                          key_fingerprint, managed_paths_json, managed_env_keys_json,
                                          expected_fields_json, orphan, apply_record_id, applied_at)
             VALUES ('zcode', 'site-1', '站点一', 'm', NULL, 'fp', '[]', '[]', '{}', 0, NULL, 1)",
            [],
        )
        .unwrap();

        crate::db::apply_schema_with_crypto(
            &conn,
            Some(&crypto),
            crate::db::migrate::BackupMode::Skip,
        )
        .unwrap();

        assert_eq!(
            count(&conn, "SELECT COUNT(*) FROM target_bindings WHERE target='zcode'"),
            0,
            "老库走 legacy 分支时也要被清洗"
        );
    }

    #[test]
    fn mounted_on_the_fresh_schema_branch() {
        // user_version=0、非 legacy：apply_schema 走「建表 + 增量补齐 + 写版本号」分支。
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE settings (id INTEGER PRIMARY KEY CHECK (id = 1), json TEXT NOT NULL);
             INSERT INTO settings (id, json) VALUES (1, '{\"localProxyTargets\":[\"z_code\"]}');
             PRAGMA user_version = 0;",
        )
        .unwrap();

        crate::db::apply_schema(&conn).unwrap();

        assert!(
            !stored(&conn, "SELECT json FROM settings WHERE id = 1").contains("z_code"),
            "补齐分支也要挂"
        );
    }

    #[test]
    fn a_cleaning_failure_never_blocks_opening_the_database() {
        // 挂载点必须把清洗失败降级成 warn：让 Db::open 报错等于让用户的应用打不开。
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA user_version = 3;").unwrap();
        conn.execute_batch(
            "CREATE TABLE settings (id INTEGER PRIMARY KEY CHECK (id = 1), json TEXT NOT NULL);
             INSERT INTO settings (id, json) VALUES (1, '{\"localProxyTargets\":[\"claude_code\"]}');
             CREATE TABLE sites (id TEXT PRIMARY KEY, zcode_api_type TEXT NOT NULL);
             INSERT INTO sites (id, zcode_api_type) VALUES ('site-1', 'anthropic-messages');",
        )
        .unwrap();

        crate::db::apply_schema(&conn).unwrap();

        assert_eq!(
            count(
                &conn,
                "SELECT COUNT(*) FROM sites WHERE zcode_api_type IS NOT NULL"
            ),
            1,
            "失败的那一步确实没写成"
        );
    }

    // ============================================================
    // AC5：~/.zcode 落盘痕迹的一次性撤退
    // ============================================================

    fn write(dir: &Path, rel: &str, content: &str) -> PathBuf {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&path, content).unwrap();
        path
    }

    fn read_back(path: &Path) -> String {
        fs::read_to_string(path).unwrap()
    }

    struct Fixture {
        root: PathBuf,
        backup: PathBuf,
        _temp: tempfile::TempDir,
    }

    fn fixture() -> Fixture {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("zcode");
        let backup = temp.path().join("backups");
        fs::create_dir_all(&root).unwrap();
        Fixture {
            root,
            backup,
            _temp: temp,
        }
    }

    fn clean(fx: &Fixture) -> FileReport {
        super::clean_external_in(&fx.root, &fx.backup)
    }

    #[test]
    fn managed_provider_entries_are_pruned_and_user_entries_survive() {
        let fx = fixture();
        let provider = write(
            &fx.root,
            PATH_PROVIDER,
            r#"{"provider":{"xiaobai_site1":{"name":"站点一","options":{"apiKey":"sk-xiaobai"}},"mine":{"name":"用户的"}},"other":1}"#,
        );
        let rules = write(
            &fx.root,
            PATH_PROVIDER_RULES,
            r#"{"schemaVersion":1,"config":{"providerOrder":["new-provider","xiaobai_site1"],"providerConfigRules":{"providerRules":[{"providerId":"mine"},{"providerId":"xiaobai_site1"}]},"modelConfigRules":{"providerModelRules":[{"modelId":"m","providerId":"xiaobai_site1"},{"modelId":"m2","providerId":"mine"}]}}}"#,
        );
        let mcp = write(
            &fx.root,
            PATH_MCP,
            r#"{"mcp":{"servers":{"xiaobai_demo":{"command":"x"},"mine":{"command":"y"}}},"other":2}"#,
        );

        let report = clean(&fx);
        assert_eq!(report.written.len(), 3, "三个落点都该被清理：{report:?}");
        assert!(report.skipped.is_empty(), "合法形状不该被跳过：{report:?}");

        let provider_after: Value = serde_json::from_str(&read_back(&provider)).unwrap();
        let keys: Vec<String> = provider_after["provider"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        assert_eq!(keys, vec!["mine".to_string()]);
        assert_eq!(provider_after["other"], json!(1), "未知字段必须原样透传");

        let rules_after: Value = serde_json::from_str(&read_back(&rules)).unwrap();
        assert_eq!(rules_after["schemaVersion"], json!(1));
        assert_eq!(
            rules_after["config"]["providerOrder"],
            json!(["new-provider"]),
            "托管项要掉，用户的 providerOrder 项要留且保持顺序"
        );
        assert_eq!(
            rules_after["config"]["providerConfigRules"]["providerRules"],
            json!([{"providerId": "mine"}])
        );
        assert_eq!(
            rules_after["config"]["modelConfigRules"]["providerModelRules"],
            json!([{"modelId": "m2", "providerId": "mine"}])
        );

        let mcp_after: Value = serde_json::from_str(&read_back(&mcp)).unwrap();
        assert_eq!(
            mcp_after["mcp"]["servers"]
                .as_object()
                .unwrap()
                .keys()
                .collect::<Vec<_>>(),
            vec!["mine"]
        );
        assert_eq!(mcp_after["other"], json!(2));

        assert!(
            fs::read_dir(&fx.backup).unwrap().count() > 0,
            "写前必须备份"
        );
    }

    #[test]
    fn managed_block_is_stripped_and_user_bytes_are_kept() {
        let fx = fixture();
        let head = "# 我自己的约定\n\n正文段落\n";
        let agents = write(
            &fx.root,
            "AGENTS.md",
            &format!("{head}\n{RULE_BEGIN}\n生成的提示\n{RULE_END}\n用户尾部\n"),
        );
        let claude = write(
            &fx.root,
            "CLAUDE.md",
            &format!("{BOM}{head}{RULE_BEGIN}\n生成的提示\n{RULE_END}\n"),
        );

        let report = clean(&fx);
        assert_eq!(report.written.len(), 2, "两份都该被清理：{report:?}");

        let agents_after = read_back(&agents);
        assert!(agents_after.starts_with(head), "块外用户内容要逐字节保留");
        assert!(agents_after.ends_with("用户尾部\n"));
        assert!(!agents_after.contains(RULE_BEGIN) && !agents_after.contains(RULE_END));
        assert!(!agents_after.contains("生成的提示"), "块内内容要整段移走");

        let claude_after = fs::read(&claude).unwrap();
        assert_eq!(&claude_after[..3], &[0xef, 0xbb, 0xbf], "UTF-8 BOM 必须保住");
        let claude_text = String::from_utf8(claude_after).unwrap();
        assert_eq!(
            claude_text.matches('\u{feff}').count(),
            1,
            "BOM 不得在清理过程中被翻倍写进用户内容"
        );
        assert!(claude_text.ends_with("正文段落\n"));
        assert!(!claude_text.contains(RULE_BEGIN));
    }

    #[test]
    fn a_file_that_is_entirely_ours_is_deleted() {
        let fx = fixture();
        let agents = write(
            &fx.root,
            "AGENTS.md",
            &format!("{RULE_BEGIN}\n只有我们的内容\n{RULE_END}\n"),
        );
        clean(&fx);
        assert!(
            !agents.exists(),
            "整份都是本应用写的文件要删掉，否则空的 AGENTS.md 会遮蔽用户自己的 CLAUDE.md"
        );
    }

    #[test]
    fn malformed_targets_are_skipped_and_left_untouched() {
        let fx = fixture();
        let agents = write(
            &fx.root,
            "AGENTS.md",
            &format!("{RULE_BEGIN}\n只有开始没有结束\n"),
        );
        let mcp = write(&fx.root, PATH_MCP, r#"{"mcp":{"servers":["xiaobai_demo"]}}"#);
        let provider = write(&fx.root, PATH_PROVIDER, r#"{"provider":"nope"}"#);

        let report = clean(&fx);

        assert_eq!(report.written.len(), 0, "非法形状一律不得被覆盖：{report:?}");
        assert_eq!(report.skipped.len(), 3, "三个非法文件都该被记录：{report:?}");
        assert!(read_back(&agents).contains(RULE_BEGIN));
        assert_eq!(read_back(&mcp), r#"{"mcp":{"servers":["xiaobai_demo"]}}"#);
        assert_eq!(read_back(&provider), r#"{"provider":"nope"}"#);
        assert!(
            !fx.backup.exists() || fs::read_dir(&fx.backup).unwrap().count() == 0,
            "没写盘就不该留下备份痕迹"
        );
    }

    #[test]
    fn nothing_is_written_when_there_is_nothing_of_ours() {
        let fx = fixture();
        let provider = write(&fx.root, PATH_PROVIDER, r#"{"provider":{"mine":{}}}"#);
        let agents = write(&fx.root, "AGENTS.md", "# 用户的文件\n\n\n");
        let before_provider = read_back(&provider);
        let before_agents = read_back(&agents);

        let report = clean(&fx);

        assert!(report.is_no_op(), "无托管内容时不该有任何动作：{report:?}");
        assert_eq!(read_back(&provider), before_provider);
        assert_eq!(read_back(&agents), before_agents, "顺手整理用户的空行也是改动");
        assert!(!fx.backup.exists(), "不写盘就不建备份目录");
    }

    #[test]
    fn file_retirement_is_idempotent() {
        let fx = fixture();
        write(
            &fx.root,
            PATH_PROVIDER,
            r#"{"provider":{"xiaobai_site1":{"name":"站点一"},"mine":{}}}"#,
        );
        write(
            &fx.root,
            "AGENTS.md",
            &format!("用户\n{RULE_BEGIN}\n约束\n{RULE_END}\n"),
        );

        assert!(!clean(&fx).is_no_op(), "首轮必须有动作");
        let second = clean(&fx);
        assert!(second.is_no_op(), "第二轮必须 no-op：{second:?}");
    }

    #[test]
    fn a_missing_zcode_directory_is_completely_silent() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("definitely-not-zcode");
        let backup = temp.path().join("backups");

        let report = super::clean_external_in(&root, &backup);

        assert!(report.is_no_op(), "目录不存在时必须全程静默：{report:?}");
        assert!(report.skipped.is_empty(), "静默 = 不记 warn：{report:?}");
        assert!(!root.exists(), "不得创建用户目录");
        assert!(!backup.exists(), "不得创建备份目录");
    }

    #[test]
    fn the_file_side_only_ever_touches_the_injected_root() {
        // 落盘用例全部走 tempdir：这条测试确认「注入根目录不存在时什么都不会发生」，
        // 而不是硬写死真实目录。
        let fx = fixture();
        let report = super::clean_external_in(&fx.root, &fx.backup);
        assert!(report.is_no_op());
        assert!(fx.root.exists());
        assert!(!fx.backup.exists());
    }

    #[test]
    fn legacy_override_key_survives_the_scrub() {
        // 文件侧清理靠这个键找回「用户配过自定义 ZCode 目录」那批人。清洗只动数组元素，
        // 把这个键一起冲掉就会让那台机器上的明文 key 永久残留。
        let conn = v15_conn();
        assert_eq!(
            super::leftover_home_override(&conn).unwrap().as_deref(),
            Some("D:/custom/zcode"),
            "夹具里的自定义目录键必须读得出来"
        );

        apply_clean(&conn);

        assert_eq!(
            super::leftover_home_override(&conn).unwrap().as_deref(),
            Some("D:/custom/zcode"),
            "清洗后仍要读得出来：启动顺序是先清洗、后清理文件"
        );
    }

    #[test]
    fn a_missing_or_blank_legacy_override_reads_as_none() {
        let conn = schema_conn();
        assert_eq!(super::leftover_home_override(&conn).unwrap(), None);

        let mut settings = serde_json::to_value(AppSettings::default()).unwrap();
        settings.as_object_mut().unwrap().insert(
            LEGACY_OVERRIDE_KEY.to_string(),
            json!("   "),
        );
        conn.execute(
            "INSERT INTO settings (id, json) VALUES (1, ?1)
             ON CONFLICT(id) DO UPDATE SET json = excluded.json",
            params![settings.to_string()],
        )
        .unwrap();
        assert_eq!(
            super::leftover_home_override(&conn).unwrap(),
            None,
            "空白 override 不得当成一个真实目录"
        );
    }

    #[test]
    fn the_same_root_is_only_cleaned_once() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_path_buf();
        let mut roots: Vec<PathBuf> = Vec::new();

        super::merge_root(&mut roots, root.clone());
        super::merge_root(&mut roots, root.join("."));
        super::merge_root(&mut roots, root.clone());

        assert_eq!(roots.len(), 1, "同一目录的两种写法不该清两遍：{roots:?}");

        super::merge_root(&mut roots, temp.path().join("another"));
        assert_eq!(roots.len(), 2, "不同目录必须都留下");
    }
}
