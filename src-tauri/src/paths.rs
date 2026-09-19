use crate::error::{AppError, AppResult};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Once;
use std::time::{Duration, SystemTime};

/// 当前应用数据目录名。
pub const APP_DIR_NAME: &str = ".xiaobai-switch";
/// AnySwitch 时期的数据目录名，仅用于一次性迁移、新旧数据接管与回滚。
pub const LEGACY_APP_DIR_NAME: &str = ".any-switch";
/// 当前数据库文件名。
pub const DB_FILE_NAME: &str = "xiaobai-switch.db";
/// AnySwitch 时期的数据库文件名，仅用于迁移识别。
pub const LEGACY_DB_FILE_NAME: &str = "any-switch.db";
const DB_SIDECAR_SUFFIXES: [&str; 3] = ["-wal", "-shm", "-journal"];
/// 两个目录同时存在时，数据库修改时间差超过该值才视为「旧目录是更新的那份数据」。
const TAKEOVER_MTIME_THRESHOLD: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppPaths {
    pub app_dir: String,
    pub db_path: String,
    pub master_key_path: String,
    pub backups_dir: String,
    pub app_backups_dir: String,
    pub codex_env_path: String,
    pub logs_dir: String,
}

pub fn home_dir() -> AppResult<PathBuf> {
    dirs::home_dir().ok_or_else(|| AppError::new("internal", "cannot resolve home directory"))
}

/// 数据目录覆盖（测试/多实例隔离）：优先 `XIAOBAI_SWITCH_DATA_DIR`，
/// 兼容 AnySwitch 时期的 `ANY_SWITCH_DATA_DIR`。
fn override_from(lookup: impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
    for key in ["XIAOBAI_SWITCH_DATA_DIR", "ANY_SWITCH_DATA_DIR"] {
        if let Some(dir) = lookup(key) {
            let trimmed = dir.trim();
            if !trimmed.is_empty() {
                return Some(PathBuf::from(trimmed));
            }
        }
    }
    None
}

pub fn app_dir() -> AppResult<PathBuf> {
    if let Some(dir) = override_from(|key| std::env::var(key).ok()) {
        return Ok(dir);
    }
    let home = home_dir()?;
    resolve_app_dir(&home.join(APP_DIR_NAME), &home.join(LEGACY_APP_DIR_NAME))
}

/// 决定当前数据目录，并在必要时迁移 / 接管旧目录。
///
/// 迁移采用 **复制**（绝不移动/删除旧目录）：`~/.any-switch` 会原样保留，
/// 作为用户回滚到旧版本时的数据来源。复制后校验数据库 sha256 与 `master.key`
/// 字节，任一失败则回退使用旧目录，应用仍可正常启动。
///
/// 改名前后两个目录（`.xiaobai-switch` / `.any-switch`）可能同时存在，且**旧目录
/// 里可能是更新的数据**（AnySwitch 时期一直在用）。此时按数据库最新修改时间比较：
/// 旧目录明显更新（超过 [`TAKEOVER_MTIME_THRESHOLD`]）时先把现有新目录改名为
/// `.pre-adopt-<unix秒>` 备份（绝不删除），再从旧目录复制接管；否则保持使用新目录。
fn resolve_app_dir(new_dir: &Path, legacy_dir: &Path) -> AppResult<PathBuf> {
    let new_exists = new_dir.exists();
    let legacy_exists = legacy_dir.exists();

    if !new_exists && legacy_exists {
        match migrate_legacy_dir(legacy_dir, new_dir) {
            Ok(()) => {
                rename_legacy_database_files(new_dir);
                return Ok(new_dir.to_path_buf());
            }
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    legacy = %legacy_dir.display(),
                    "failed to migrate legacy app data; falling back to the legacy directory"
                );
                return Ok(legacy_dir.to_path_buf());
            }
        }
    }

    if new_exists && legacy_exists {
        if legacy_dir_is_newer(new_dir, legacy_dir) {
            match take_over_new_dir(new_dir, legacy_dir) {
                Ok(()) => return Ok(new_dir.to_path_buf()),
                Err(error) => {
                    tracing::warn!(
                        error = %error,
                        newer = %legacy_dir.display(),
                        "failed to take over the newer legacy app data; falling back to the legacy directory"
                    );
                    return Ok(legacy_dir.to_path_buf());
                }
            }
        }
        static WARN_ONCE: Once = Once::new();
        WARN_ONCE.call_once(|| {
            tracing::warn!(
                legacy = %legacy_dir.display(),
                "legacy app data directory is still present; using the current directory and leaving it untouched"
            );
        });
    }

    if new_exists {
        rename_legacy_database_files(new_dir);
    }
    Ok(new_dir.to_path_buf())
}

/// 目录内数据库（含 `-wal/-shm/-journal` 旁文件）的最新修改时间。
///
/// 两个目录可能各自使用不同的库名（新目录 `xiaobai-switch.db`、旧目录
/// `any-switch.db`，也可能因迁移中断而混用），所以两种名字都要看。
fn latest_database_mtime(dir: &Path) -> Option<SystemTime> {
    let mut latest: Option<SystemTime> = None;
    for name in [DB_FILE_NAME, LEGACY_DB_FILE_NAME] {
        for suffix in std::iter::once("").chain(DB_SIDECAR_SUFFIXES) {
            let path = dir.join(format!("{name}{suffix}"));
            let Ok(metadata) = fs::metadata(&path) else {
                continue;
            };
            if !metadata.is_file() {
                continue;
            }
            let Ok(modified) = metadata.modified() else {
                continue;
            };
            latest = Some(match latest {
                Some(current) => current.max(modified),
                None => modified,
            });
        }
    }
    latest
}

/// 旧目录是否装着明显更新的数据，需要被接管。
fn legacy_dir_is_newer(new_dir: &Path, legacy_dir: &Path) -> bool {
    match (latest_database_mtime(new_dir), latest_database_mtime(legacy_dir)) {
        (Some(current), Some(legacy)) => legacy > current + TAKEOVER_MTIME_THRESHOLD,
        // 新目录里没有可用的数据库、旧目录有：旧目录显然更完整。
        (None, Some(_)) => true,
        _ => false,
    }
}

/// 旧目录明显更新时安全接管新目录名：先把现有新目录改名为同级
/// `.pre-adopt-<unix秒>`（绝不删除），再从旧目录复制迁移。
///
/// 迁移失败时把备份目录改名回新目录名并返回错误；调用方会回退使用旧目录，
/// 应用仍可启动。旧目录本身始终原样保留。
fn take_over_new_dir(new_dir: &Path, legacy_dir: &Path) -> AppResult<()> {
    let backup = new_dir.with_file_name(format!(
        "{}.pre-adopt-{}",
        new_dir
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("app-data"),
        chrono::Utc::now().timestamp()
    ));
    fs::rename(new_dir, &backup)?;
    match migrate_legacy_dir(legacy_dir, new_dir) {
        Ok(()) => {
            rename_legacy_database_files(new_dir);
            tracing::info!(
                adopted = %legacy_dir.display(),
                replaced_backup = %backup.display(),
                "adopted the newer legacy app data directory"
            );
            Ok(())
        }
        Err(error) => {
            if let Err(restore_error) = fs::rename(&backup, new_dir) {
                // 无法恢复原名时数据仍留在 pre-adopt 目录里，绝不删除。
                tracing::error!(
                    restore_error = %restore_error,
                    backup = %backup.display(),
                    "failed to restore the current app data directory after a failed takeover"
                );
            }
            Err(error)
        }
    }
}

/// 复制旧目录到新目录并校验；保留旧目录。
///
/// 先复制到同级的唯一暂存目录，校验通过后再原子 rename 到 `new_dir`。这样进程若在
/// 复制中途被杀死，只会留下带 pid/uuid 后缀的暂存目录，下次启动会清理并重新迁移，
/// 不会把半份数据当成正式数据目录使用，也不会误提并发的半成品。
fn migrate_legacy_dir(legacy_dir: &Path, new_dir: &Path) -> AppResult<()> {
    if new_dir.exists() {
        return Err(AppError::new(
            "internal",
            "new app data directory already exists",
        ));
    }
    let prefix = migration_staging_prefix(new_dir);
    sweep_stale_migration_staging(new_dir, &prefix);
    let staging = new_dir.with_file_name(format!(
        "{prefix}-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4().simple()
    ));
    let copied = (|| -> AppResult<()> {
        fs::create_dir_all(&staging)?;
        copy_dir_contents(legacy_dir, &staging)?;
        verify_migrated_dir(legacy_dir, &staging)
    })();
    if let Err(error) = copied {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }
    if let Err(error) = fs::rename(&staging, new_dir) {
        let _ = fs::remove_dir_all(&staging);
        return Err(error.into());
    }
    Ok(())
}

fn migration_staging_prefix(new_dir: &Path) -> String {
    format!(
        "{}.migrating",
        new_dir
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("app-data")
    )
}

/// 清理上次被强杀留下的暂存目录（仅限本目标目录的 `.migrating*` 同名兄弟）。
fn sweep_stale_migration_staging(new_dir: &Path, prefix: &str) {
    let Some(parent) = new_dir.parent() else {
        return;
    };
    let Ok(entries) = fs::read_dir(parent) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if name.starts_with(prefix) && entry.path().is_dir() {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

fn copy_dir_contents(from: &Path, to: &Path) -> AppResult<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let target = to.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_contents(&entry.path(), &target)?;
        } else if file_type.is_file() {
            fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

fn verify_migrated_dir(legacy_dir: &Path, new_dir: &Path) -> AppResult<()> {
    let mut migrated_database: Option<&str> = None;
    for name in [DB_FILE_NAME, LEGACY_DB_FILE_NAME] {
        let source = legacy_dir.join(name);
        if !source.is_file() {
            continue;
        }
        let target = new_dir.join(name);
        if !target.is_file() || sha256_file(&target)? != sha256_file(&source)? {
            return Err(AppError::new(
                "internal",
                format!("migrated database {name} failed checksum verification"),
            ));
        }
        // WAL 里可能有未 checkpoint 的已提交数据：旁文件也必须逐字节一致。
        for suffix in DB_SIDECAR_SUFFIXES {
            let source_sidecar = legacy_dir.join(format!("{name}{suffix}"));
            if !source_sidecar.is_file() {
                continue;
            }
            let target_sidecar = new_dir.join(format!("{name}{suffix}"));
            if !target_sidecar.is_file()
                || sha256_file(&target_sidecar)? != sha256_file(&source_sidecar)?
            {
                return Err(AppError::new(
                    "internal",
                    format!("migrated database sidecar {name}{suffix} failed checksum verification"),
                ));
            }
        }
        if migrated_database.is_none() {
            migrated_database = Some(name);
        }
    }
    // 字节一致不足以保证可读（例如源库在复制期间被 checkpoint）：真实打开一次。
    if let Some(name) = migrated_database {
        verify_database_opens(&new_dir.join(name))?;
    }
    let source_key = legacy_dir.join("master.key");
    if source_key.is_file() {
        let target_key = new_dir.join("master.key");
        let source_bytes = fs::read(&source_key)?;
        let target_bytes = fs::read(&target_key)?;
        if source_bytes.len() != 32 || target_bytes != source_bytes {
            return Err(AppError::new(
                "internal",
                "migrated master.key failed verification",
            ));
        }
    }
    Ok(())
}

/// 打开迁移后的数据库并做 `PRAGMA quick_check`，确保不是半份/损坏的副本。
fn verify_database_opens(path: &Path) -> AppResult<()> {
    let conn = rusqlite::Connection::open(path).map_err(|error| {
        AppError::new(
            "internal",
            format!("migrated database cannot be opened: {error}"),
        )
    })?;
    let check: String = conn
        .query_row("PRAGMA quick_check", [], |row| row.get(0))
        .map_err(|error| {
            AppError::new(
                "internal",
                format!("migrated database integrity check failed: {error}"),
            )
        })?;
    if check != "ok" {
        return Err(AppError::new(
            "internal",
            format!("migrated database integrity check failed: {check}"),
        ));
    }
    Ok(())
}

/// 目录内仅有旧数据库文件名时重命名为新名字（含 WAL/SHM/JOURNAL 旁文件）。
///
/// 主库已改名不代表改名完成：进程可能恰好死在「主库已改名、旁文件还没改名」
/// 之间。此时若直接返回，旧名的 WAL 就成了孤儿 —— SQLite 打开新名主库时不会
/// 读它，其中**已提交但未 checkpoint 的数据会被静默忽略**。因此主库存在时
/// 仍要把遗留的旧名旁文件补改名（新名旁文件已存在则不动，避免覆盖新数据）。
fn rename_legacy_database_files(dir: &Path) {
    let new_db = dir.join(DB_FILE_NAME);
    if !new_db.exists() {
        let legacy_db = dir.join(LEGACY_DB_FILE_NAME);
        if !legacy_db.is_file() {
            rename_legacy_database_sidecars(dir);
            return;
        }
        if let Err(error) = fs::rename(&legacy_db, &new_db) {
            tracing::warn!(
                error = %error,
                "failed to rename the legacy database file; keeping the legacy name"
            );
            return;
        }
    }
    rename_legacy_database_sidecars(dir);
}

fn rename_legacy_database_sidecars(dir: &Path) {
    for suffix in DB_SIDECAR_SUFFIXES {
        let from = dir.join(format!("{LEGACY_DB_FILE_NAME}{suffix}"));
        if !from.is_file() {
            continue;
        }
        let to = dir.join(format!("{DB_FILE_NAME}{suffix}"));
        if to.exists() {
            continue;
        }
        let _ = fs::rename(from, to);
    }
}

pub fn ensure_app_dirs() -> AppResult<PathBuf> {
    let dir = app_dir()?;
    fs::create_dir_all(&dir)?;
    fs::create_dir_all(dir.join("backups"))?;
    fs::create_dir_all(dir.join("backups").join("app"))?;
    fs::create_dir_all(dir.join("env"))?;
    fs::create_dir_all(dir.join("locks"))?;
    fs::create_dir_all(dir.join("logs"))?;
    Ok(dir)
}

/// 解析目录内实际使用的数据库文件名：优先新名字，迁移回退时可能是旧名字。
pub(crate) fn resolve_db_file_name(dir: &Path) -> &'static str {
    if dir.join(DB_FILE_NAME).exists() {
        DB_FILE_NAME
    } else if dir.join(LEGACY_DB_FILE_NAME).exists() {
        // 复制迁移失败回退旧目录时，数据库仍是旧文件名。
        LEGACY_DB_FILE_NAME
    } else {
        DB_FILE_NAME
    }
}

pub fn db_path() -> AppResult<PathBuf> {
    let dir = app_dir()?;
    Ok(dir.join(resolve_db_file_name(&dir)))
}

pub fn master_key_path() -> AppResult<PathBuf> {
    Ok(app_dir()?.join("master.key"))
}

/// 本地代理的路径口令。
///
/// 刻意放**文件**而不是 settings 表：settings 会随 WebDAV 同步到别的机器，而口令
/// 必须与本机的 CLI 配置一致（配置由各机器分别写入）。同步来的口令会让本机代理
/// 监听的口令与客户端配置里的口令不一致，表现为四个 CLI 全部 404。
pub fn local_proxy_token_path() -> AppResult<PathBuf> {
    Ok(app_dir()?.join("local-proxy-token"))
}

/// 本地代理的路径口令：按需生成并落盘（0600）。
///
/// 生成一次后跨重启稳定：CLI 配置里写的是它，换了它所有客户端都要重写。
pub fn ensure_local_proxy_token() -> AppResult<String> {
    let path = local_proxy_token_path()?;
    if let Ok(existing) = std::fs::read_to_string(&path) {
        let trimmed = existing.trim().to_ascii_lowercase();
        if crate::domain::is_valid_local_proxy_token(&trimmed) {
            return Ok(trimmed);
        }
    }
    let token = crate::domain::generate_local_proxy_token();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, &token)?;
    crate::paths::set_secret_permissions(&path);
    Ok(token)
}

pub fn codex_env_path() -> AppResult<PathBuf> {
    Ok(app_dir()?.join("env").join("codex.env"))
}

pub fn backups_dir() -> AppResult<PathBuf> {
    Ok(app_dir()?.join("backups"))
}

pub fn app_backups_dir() -> AppResult<PathBuf> {
    Ok(backups_dir()?.join("app"))
}

pub fn locks_dir() -> AppResult<PathBuf> {
    Ok(app_dir()?.join("locks"))
}

pub fn default_claude_home() -> AppResult<PathBuf> {
    Ok(home_dir()?.join(".claude"))
}

pub fn default_codex_home() -> AppResult<PathBuf> {
    if let Ok(v) = std::env::var("CODEX_HOME") {
        if !v.trim().is_empty() {
            return Ok(PathBuf::from(v));
        }
    }
    Ok(home_dir()?.join(".codex"))
}

pub fn default_pi_agent_dir() -> AppResult<PathBuf> {
    if let Ok(v) = std::env::var("PI_CODING_AGENT_DIR") {
        if !v.trim().is_empty() {
            return Ok(PathBuf::from(v));
        }
    }
    Ok(home_dir()?.join(".pi").join("agent"))
}

pub fn default_prime_agent_dir() -> AppResult<PathBuf> {
    if let Ok(v) = std::env::var("PRIME_AGENT_CODING_AGENT_DIR") {
        if !v.trim().is_empty() {
            return Ok(PathBuf::from(v));
        }
    }
    Ok(home_dir()?.join(".prime").join("agent"))
}

pub fn resolve_claude_home(override_path: Option<&str>) -> AppResult<PathBuf> {
    if let Some(p) = override_path {
        if !p.trim().is_empty() {
            return Ok(PathBuf::from(p));
        }
    }
    default_claude_home()
}

/// Claude Code 的用户级 MCP 定义位于 `~/.claude.json` 顶层的 `mcpServers`。
/// `~/.claude/settings.json` 里的同名键会被 Claude Code 忽略，因此 MCP 必须写这个文件。
/// 设置了 `CLAUDE_CONFIG_DIR` 时该文件位于该目录内部。
pub fn claude_mcp_json_path(claude_home_override: Option<&str>) -> AppResult<PathBuf> {
    if let Some(p) = claude_home_override {
        if !p.trim().is_empty() {
            return Ok(PathBuf::from(p).join(".claude.json"));
        }
    }
    if let Ok(v) = std::env::var("CLAUDE_CONFIG_DIR") {
        if !v.trim().is_empty() {
            return Ok(PathBuf::from(v).join(".claude.json"));
        }
    }
    Ok(home_dir()?.join(".claude.json"))
}

/// Claude Code 的用户级记忆文件。设置 `CLAUDE_CONFIG_DIR` 时整个 `~/.claude` 都会
/// 搬到该目录下，记忆文件随之变成该目录内的 `CLAUDE.md`，所以这里与
/// `claude_mcp_json_path` 用同一套优先级：应用内 override → 环境变量 → 家目录。
pub fn claude_rules_path(claude_home_override: Option<&str>) -> AppResult<PathBuf> {
    if let Some(p) = claude_home_override {
        if !p.trim().is_empty() {
            return Ok(PathBuf::from(p).join("CLAUDE.md"));
        }
    }
    if let Ok(v) = std::env::var("CLAUDE_CONFIG_DIR") {
        if !v.trim().is_empty() {
            return Ok(PathBuf::from(v).join("CLAUDE.md"));
        }
    }
    Ok(default_claude_home()?.join("CLAUDE.md"))
}

pub fn resolve_codex_home(override_path: Option<&str>) -> AppResult<PathBuf> {
    if let Some(p) = override_path {
        if !p.trim().is_empty() {
            return Ok(PathBuf::from(p));
        }
    }
    default_codex_home()
}

pub fn resolve_pi_agent_dir(override_path: Option<&str>) -> AppResult<PathBuf> {
    if let Some(p) = override_path {
        if !p.trim().is_empty() {
            return Ok(PathBuf::from(p));
        }
    }
    default_pi_agent_dir()
}

pub fn resolve_prime_agent_dir(override_path: Option<&str>) -> AppResult<PathBuf> {
    if let Some(p) = override_path {
        if !p.trim().is_empty() {
            return Ok(PathBuf::from(p));
        }
    }
    default_prime_agent_dir()
}

pub fn app_paths_dto() -> AppResult<AppPaths> {
    let dir = app_dir()?;
    Ok(AppPaths {
        app_dir: dir.display().to_string(),
        db_path: db_path()?.display().to_string(),
        master_key_path: master_key_path()?.display().to_string(),
        backups_dir: backups_dir()?.display().to_string(),
        app_backups_dir: app_backups_dir()?.display().to_string(),
        codex_env_path: codex_env_path()?.display().to_string(),
        logs_dir: dir.join("logs").display().to_string(),
    })
}

fn sha256_file(path: &Path) -> AppResult<String> {
    use std::io::Read;
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

#[cfg(unix)]
pub fn set_secret_permissions(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = fs::metadata(path) {
        let mut perms = meta.permissions();
        perms.set_mode(0o600);
        let _ = fs::set_permissions(path, perms);
    }
}

#[cfg(unix)]
pub fn set_private_dir_permissions(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = fs::metadata(path) {
        let mut perms = meta.permissions();
        perms.set_mode(0o700);
        let _ = fs::set_permissions(path, perms);
    }
}

#[cfg(not(unix))]
pub fn set_secret_permissions(_path: &std::path::Path) {}

#[cfg(not(unix))]
pub fn set_private_dir_permissions(_path: &std::path::Path) {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn write_file(path: &Path, bytes: &[u8]) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, bytes).unwrap();
    }

    /// 迁移校验会真实打开数据库做 `PRAGMA quick_check`，测试必须用真正的 SQLite 文件。
    fn write_sqlite_file(path: &Path) {
        write_sqlite_file_with(path, "ok");
    }

    fn write_sqlite_file_with(path: &Path, note: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let conn = rusqlite::Connection::open(path).unwrap();
        conn.execute_batch(&format!(
            "CREATE TABLE probe (id INTEGER PRIMARY KEY, note TEXT); \
             INSERT INTO probe (note) VALUES ('{note}');"
        ))
        .unwrap();
    }

    /// 显式设置修改时间，避免依赖文件系统精度来区分两份数据的先后。
    fn set_mtime(path: &Path, modified: SystemTime) {
        fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(modified)
            .unwrap();
    }

    fn pre_adopt_dirs(parent: &Path) -> Vec<PathBuf> {
        let prefix = format!("{APP_DIR_NAME}.pre-adopt-");
        let mut found = fs::read_dir(parent)
            .unwrap()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(&prefix))
            })
            .collect::<Vec<_>>();
        found.sort();
        found
    }

    fn write_sidecar(path: &Path) {
        write_file(path, b"sidecar-bytes");
    }

    #[test]
    fn pi_override_beats_environment() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("PI_CODING_AGENT_DIR", "/tmp/pi-env");
        assert_eq!(
            resolve_pi_agent_dir(Some("/tmp/pi-setting")).unwrap(),
            PathBuf::from("/tmp/pi-setting")
        );
        assert_eq!(
            resolve_pi_agent_dir(None).unwrap(),
            PathBuf::from("/tmp/pi-env")
        );
        std::env::remove_var("PI_CODING_AGENT_DIR");
        assert!(default_pi_agent_dir().unwrap().ends_with(".pi/agent"));
    }

    #[test]
    fn prime_override_beats_environment() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("PRIME_AGENT_CODING_AGENT_DIR", "/tmp/prime-env");
        assert_eq!(
            resolve_prime_agent_dir(Some("/tmp/prime-setting")).unwrap(),
            PathBuf::from("/tmp/prime-setting")
        );
        assert_eq!(
            resolve_prime_agent_dir(None).unwrap(),
            PathBuf::from("/tmp/prime-env")
        );
        std::env::remove_var("PRIME_AGENT_CODING_AGENT_DIR");
        assert!(default_prime_agent_dir().unwrap().ends_with(".prime/agent"));
    }

    #[test]
    fn claude_mcp_path_follows_override_then_config_dir_then_home() {
        // Claude Code 的用户级 MCP 在 ~/.claude.json；设了 CLAUDE_CONFIG_DIR 时
        // 它读的是该目录里的 .claude.json，所以三条分支都要走对。
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("CLAUDE_CONFIG_DIR");

        assert_eq!(
            claude_mcp_json_path(Some("/tmp/claude-setting")).unwrap(),
            PathBuf::from("/tmp/claude-setting").join(".claude.json")
        );

        std::env::set_var("CLAUDE_CONFIG_DIR", "/tmp/claude-env");
        assert_eq!(
            claude_mcp_json_path(None).unwrap(),
            PathBuf::from("/tmp/claude-env").join(".claude.json")
        );
        // 应用内设置优先于环境变量。
        assert_eq!(
            claude_mcp_json_path(Some("/tmp/claude-setting")).unwrap(),
            PathBuf::from("/tmp/claude-setting").join(".claude.json")
        );

        std::env::remove_var("CLAUDE_CONFIG_DIR");
        assert!(claude_mcp_json_path(None)
            .unwrap()
            .ends_with(".claude.json"));
    }

    #[test]
    fn data_dir_override_prefers_the_new_variable() {
        let map = |key: &str| match key {
            "XIAOBAI_SWITCH_DATA_DIR" => Some("  /tmp/xiaobai  ".to_string()),
            "ANY_SWITCH_DATA_DIR" => Some("/tmp/legacy".to_string()),
            _ => None,
        };
        assert_eq!(
            override_from(map).unwrap(),
            PathBuf::from("/tmp/xiaobai"),
            "new variable must win and be trimmed"
        );

        let legacy_only = |key: &str| match key {
            "ANY_SWITCH_DATA_DIR" => Some("/tmp/legacy".to_string()),
            _ => None,
        };
        assert_eq!(
            override_from(legacy_only).unwrap(),
            PathBuf::from("/tmp/legacy")
        );

        assert!(override_from(|_| None).is_none());
        assert!(override_from(|_| Some("   ".to_string())).is_none());
    }

    #[test]
    fn migrates_legacy_directory_by_copy_and_keeps_legacy() {
        let temp = tempfile::tempdir().unwrap();
        let legacy = temp.path().join(LEGACY_APP_DIR_NAME);
        let new_dir = temp.path().join(APP_DIR_NAME);
        write_sqlite_file(&legacy.join(LEGACY_DB_FILE_NAME));
        write_file(&legacy.join("master.key"), &[7_u8; 32]);
        write_file(&legacy.join("backups/app/old.zip"), b"zip");
        let legacy_db_before = fs::read(legacy.join(LEGACY_DB_FILE_NAME)).unwrap();

        let resolved = resolve_app_dir(&new_dir, &legacy).unwrap();

        assert_eq!(resolved, new_dir);
        assert!(
            new_dir.join(DB_FILE_NAME).is_file(),
            "database must be migrated and renamed in the new directory"
        );
        assert!(!new_dir.join(LEGACY_DB_FILE_NAME).exists());
        assert_eq!(
            fs::read(new_dir.join("master.key")).unwrap(),
            vec![7_u8; 32]
        );
        assert_eq!(
            fs::read(new_dir.join("backups/app/old.zip")).unwrap(),
            b"zip"
        );

        // 旧目录作为回滚点原样保留。
        assert!(legacy.is_dir());
        assert_eq!(
            fs::read(legacy.join(LEGACY_DB_FILE_NAME)).unwrap(),
            legacy_db_before
        );
        assert_eq!(fs::read(legacy.join("master.key")).unwrap(), vec![7_u8; 32]);
        assert!(!new_dir
            .with_file_name(format!("{APP_DIR_NAME}.migrating"))
            .exists());
    }

    #[test]
    fn recovers_from_an_interrupted_migration_without_using_partial_data() {
        let temp = tempfile::tempdir().unwrap();
        let legacy = temp.path().join(LEGACY_APP_DIR_NAME);
        let new_dir = temp.path().join(APP_DIR_NAME);
        write_sqlite_file(&legacy.join(LEGACY_DB_FILE_NAME));
        write_file(&legacy.join("master.key"), &[7_u8; 32]);
        // 模拟上次迁移被强杀：残留一个只有半个文件的暂存目录（含带 pid/uuid 后缀的）。
        write_file(
            &temp
                .path()
                .join(format!("{APP_DIR_NAME}.migrating"))
                .join("half.tmp"),
            b"half",
        );
        write_file(
            &temp
                .path()
                .join(format!("{APP_DIR_NAME}.migrating-1234-deadbeef"))
                .join("half.tmp"),
            b"half",
        );

        let resolved = resolve_app_dir(&new_dir, &legacy).unwrap();

        assert_eq!(resolved, new_dir);
        assert!(new_dir.join(DB_FILE_NAME).is_file());
        assert!(!temp
            .path()
            .join(format!("{APP_DIR_NAME}.migrating"))
            .exists());
        assert!(!temp
            .path()
            .join(format!("{APP_DIR_NAME}.migrating-1234-deadbeef"))
            .exists());
        assert!(legacy.join(LEGACY_DB_FILE_NAME).exists());
    }

    #[test]
    fn copy_and_verify_migrated_dir_includes_wal_sidecars() {
        let temp = tempfile::tempdir().unwrap();
        let legacy = temp.path().join("legacy");
        let new_dir = temp.path().join("new");
        write_sqlite_file(&legacy.join(LEGACY_DB_FILE_NAME));
        write_sidecar(&legacy.join(format!("{LEGACY_DB_FILE_NAME}-journal")));
        write_sidecar(&legacy.join(format!("{LEGACY_DB_FILE_NAME}-shm")));
        write_file(&legacy.join("master.key"), &[7_u8; 32]);

        fs::create_dir_all(&new_dir).unwrap();
        copy_dir_contents(&legacy, &new_dir).unwrap();

        assert!(
            new_dir
                .join(format!("{LEGACY_DB_FILE_NAME}-journal"))
                .is_file(),
            "sidecar files must be copied, not only the main database"
        );
        assert!(new_dir
            .join(format!("{LEGACY_DB_FILE_NAME}-shm"))
            .is_file());
        assert!(verify_migrated_dir(&legacy, &new_dir).is_ok());
    }

    #[test]
    fn verify_migrated_dir_rejects_a_tampered_database() {
        let temp = tempfile::tempdir().unwrap();
        let legacy = temp.path().join("legacy");
        let new_dir = temp.path().join("new");
        write_file(&legacy.join(LEGACY_DB_FILE_NAME), b"legacy-bytes");
        write_file(&new_dir.join(LEGACY_DB_FILE_NAME), b"tampered-bytes");

        assert!(verify_migrated_dir(&legacy, &new_dir).is_err());
    }

    #[test]
    fn verify_migrated_dir_rejects_a_tampered_wal_sidecar() {
        let temp = tempfile::tempdir().unwrap();
        let legacy = temp.path().join("legacy");
        let new_dir = temp.path().join("new");
        write_sqlite_file(&legacy.join(LEGACY_DB_FILE_NAME));
        write_sidecar(&legacy.join(format!("{LEGACY_DB_FILE_NAME}-wal")));
        // 主库字节必须一致，才能确保失败来自 wal 旁文件校验。
        fs::create_dir_all(&new_dir).unwrap();
        fs::copy(
            legacy.join(LEGACY_DB_FILE_NAME),
            new_dir.join(LEGACY_DB_FILE_NAME),
        )
        .unwrap();
        write_file(
            &new_dir.join(format!("{LEGACY_DB_FILE_NAME}-wal")),
            b"tampered-sidecar",
        );

        assert!(verify_migrated_dir(&legacy, &new_dir).is_err());
    }

    #[test]
    fn verify_migrated_dir_rejects_a_database_that_cannot_be_opened() {
        let temp = tempfile::tempdir().unwrap();
        let legacy = temp.path().join("legacy");
        let new_dir = temp.path().join("new");
        // 字节一致（绕过 checksum），但不是合法 SQLite：必须被真实打开校验拦下。
        write_file(&legacy.join(LEGACY_DB_FILE_NAME), b"not a sqlite database");
        write_file(
            &new_dir.join(LEGACY_DB_FILE_NAME),
            b"not a sqlite database",
        );

        assert!(verify_migrated_dir(&legacy, &new_dir).is_err());
    }

    #[test]
    fn prefers_new_directory_and_leaves_both_sides_untouched() {
        let temp = tempfile::tempdir().unwrap();
        let legacy = temp.path().join(LEGACY_APP_DIR_NAME);
        let new_dir = temp.path().join(APP_DIR_NAME);
        write_file(&legacy.join(LEGACY_DB_FILE_NAME), b"legacy-db");
        write_file(&new_dir.join(DB_FILE_NAME), b"new-db");
        // 旧目录明显更旧：保持原有「选新目录」行为，两边都不动。
        let old = SystemTime::now() - Duration::from_secs(3600);
        set_mtime(&legacy.join(LEGACY_DB_FILE_NAME), old);

        let resolved = resolve_app_dir(&new_dir, &legacy).unwrap();

        assert_eq!(resolved, new_dir);
        assert_eq!(fs::read(new_dir.join(DB_FILE_NAME)).unwrap(), b"new-db");
        assert_eq!(
            fs::read(legacy.join(LEGACY_DB_FILE_NAME)).unwrap(),
            b"legacy-db"
        );
        assert!(pre_adopt_dirs(temp.path()).is_empty());
    }

    #[test]
    fn uses_new_directory_when_both_databases_have_the_same_freshness() {
        let temp = tempfile::tempdir().unwrap();
        let legacy = temp.path().join(LEGACY_APP_DIR_NAME);
        let new_dir = temp.path().join(APP_DIR_NAME);
        write_file(&legacy.join(LEGACY_DB_FILE_NAME), b"legacy-db");
        write_file(&new_dir.join(DB_FILE_NAME), b"new-db");
        let same = SystemTime::now() - Duration::from_secs(60);
        set_mtime(&legacy.join(LEGACY_DB_FILE_NAME), same);
        set_mtime(&new_dir.join(DB_FILE_NAME), same);

        let resolved = resolve_app_dir(&new_dir, &legacy).unwrap();

        assert_eq!(resolved, new_dir, "a tiny mtime delta must not trigger a takeover");
        assert_eq!(fs::read(new_dir.join(DB_FILE_NAME)).unwrap(), b"new-db");
        assert_eq!(
            fs::read(legacy.join(LEGACY_DB_FILE_NAME)).unwrap(),
            b"legacy-db"
        );
        assert!(pre_adopt_dirs(temp.path()).is_empty());
    }

    #[test]
    fn takes_over_the_newer_legacy_directory_and_keeps_both_data_copies() {
        let temp = tempfile::tempdir().unwrap();
        let legacy = temp.path().join(LEGACY_APP_DIR_NAME);
        let new_dir = temp.path().join(APP_DIR_NAME);
        write_sqlite_file_with(&legacy.join(LEGACY_DB_FILE_NAME), "fresh-legacy");
        write_file(&legacy.join("master.key"), &[7_u8; 32]);
        write_file(&legacy.join("backups/app/old.zip"), b"zip");
        let legacy_db_before = fs::read(legacy.join(LEGACY_DB_FILE_NAME)).unwrap();

        write_sqlite_file_with(&new_dir.join(DB_FILE_NAME), "stale-new");
        write_file(&new_dir.join("master.key"), &[9_u8; 32]);
        let stale_new_db_before = fs::read(new_dir.join(DB_FILE_NAME)).unwrap();

        let now = SystemTime::now();
        set_mtime(&legacy.join(LEGACY_DB_FILE_NAME), now);
        set_mtime(&new_dir.join(DB_FILE_NAME), now - Duration::from_secs(3600));

        let resolved = resolve_app_dir(&new_dir, &legacy).unwrap();

        assert_eq!(resolved, new_dir);
        assert_eq!(
            fs::read(new_dir.join(DB_FILE_NAME)).unwrap(),
            legacy_db_before,
            "the newer legacy database must win"
        );
        assert_eq!(fs::read(new_dir.join("master.key")).unwrap(), vec![7_u8; 32]);
        assert_eq!(fs::read(new_dir.join("backups/app/old.zip")).unwrap(), b"zip");
        assert!(
            !new_dir.join(LEGACY_DB_FILE_NAME).exists(),
            "the adopted legacy database must be renamed to the current name"
        );

        // 旧目录必须原样保留，作为回滚点。
        assert_eq!(
            fs::read(legacy.join(LEGACY_DB_FILE_NAME)).unwrap(),
            legacy_db_before
        );
        assert_eq!(fs::read(legacy.join("master.key")).unwrap(), vec![7_u8; 32]);
        assert!(legacy.join("backups/app/old.zip").is_file());

        // 被替换的新目录以 pre-adopt-* 形式留存，内容仍在。
        let backups = pre_adopt_dirs(temp.path());
        assert_eq!(backups.len(), 1, "the replaced directory must be kept");
        assert_eq!(
            fs::read(backups[0].join(DB_FILE_NAME)).unwrap(),
            stale_new_db_before
        );
        assert_eq!(fs::read(backups[0].join("master.key")).unwrap(), vec![9_u8; 32]);
    }

    #[test]
    fn falls_back_to_the_newer_legacy_directory_when_takeover_verification_fails() {
        let temp = tempfile::tempdir().unwrap();
        let legacy = temp.path().join(LEGACY_APP_DIR_NAME);
        let new_dir = temp.path().join(APP_DIR_NAME);
        write_sqlite_file_with(&legacy.join(LEGACY_DB_FILE_NAME), "fresh-legacy");
        // 3 字节的 master.key 必然让复制校验失败。
        write_file(&legacy.join("master.key"), b"bad-key");
        let legacy_db_before = fs::read(legacy.join(LEGACY_DB_FILE_NAME)).unwrap();

        write_sqlite_file(&new_dir.join(DB_FILE_NAME));
        write_file(&new_dir.join("master.key"), &[9_u8; 32]);
        let stale_new_db_before = fs::read(new_dir.join(DB_FILE_NAME)).unwrap();

        let now = SystemTime::now();
        set_mtime(&legacy.join(LEGACY_DB_FILE_NAME), now);
        set_mtime(&new_dir.join(DB_FILE_NAME), now - Duration::from_secs(3600));

        let resolved = resolve_app_dir(&new_dir, &legacy).unwrap();

        // 接管失败：回退使用（更新的）旧目录，应用仍可启动。
        assert_eq!(resolved, legacy);
        assert_eq!(
            fs::read(legacy.join(LEGACY_DB_FILE_NAME)).unwrap(),
            legacy_db_before
        );
        // 新目录已恢复原名、内容不变；pre-adopt 备份已改名回去。
        assert_eq!(
            fs::read(new_dir.join(DB_FILE_NAME)).unwrap(),
            stale_new_db_before
        );
        assert_eq!(fs::read(new_dir.join("master.key")).unwrap(), vec![9_u8; 32]);
        assert!(pre_adopt_dirs(temp.path()).is_empty());
        assert!(!new_dir
            .with_file_name(format!("{APP_DIR_NAME}.migrating"))
            .exists());
    }

    /// 进程死在「主库已改名、旁文件未改名」之间会留下孤儿旧名 WAL：
    /// 主库已存在时也必须把旧名旁文件补改名，否则其中的已提交数据会被静默忽略。
    #[test]
    fn renames_orphaned_legacy_sidecars_when_the_primary_database_already_exists() {
        let temp = tempfile::tempdir().unwrap();
        let legacy = temp.path().join(LEGACY_APP_DIR_NAME);
        let new_dir = temp.path().join(APP_DIR_NAME);
        write_file(&new_dir.join(DB_FILE_NAME), b"db");
        write_file(&new_dir.join(format!("{LEGACY_DB_FILE_NAME}-wal")), b"orphan-wal");
        write_file(&new_dir.join(format!("{LEGACY_DB_FILE_NAME}-shm")), b"orphan-shm");

        let resolved = resolve_app_dir(&new_dir, &legacy).unwrap();

        assert_eq!(resolved, new_dir);
        assert_eq!(
            fs::read(new_dir.join(format!("{DB_FILE_NAME}-wal"))).unwrap(),
            b"orphan-wal"
        );
        assert_eq!(
            fs::read(new_dir.join(format!("{DB_FILE_NAME}-shm"))).unwrap(),
            b"orphan-shm"
        );
        assert!(!new_dir.join(format!("{LEGACY_DB_FILE_NAME}-wal")).exists());
        assert!(!new_dir.join(format!("{LEGACY_DB_FILE_NAME}-shm")).exists());
    }

    /// 新名旁文件已存在时不能覆盖：那份才是当前主库正在用的。
    #[test]
    fn keeps_the_new_sidecar_when_both_names_exist() {
        let temp = tempfile::tempdir().unwrap();
        let legacy = temp.path().join(LEGACY_APP_DIR_NAME);
        let new_dir = temp.path().join(APP_DIR_NAME);
        write_file(&new_dir.join(DB_FILE_NAME), b"db");
        write_file(&new_dir.join(format!("{DB_FILE_NAME}-wal")), b"current-wal");
        write_file(&new_dir.join(format!("{LEGACY_DB_FILE_NAME}-wal")), b"stale-wal");

        let resolved = resolve_app_dir(&new_dir, &legacy).unwrap();

        assert_eq!(resolved, new_dir);
        assert_eq!(
            fs::read(new_dir.join(format!("{DB_FILE_NAME}-wal"))).unwrap(),
            b"current-wal"
        );
    }

    #[test]
    fn renames_legacy_database_inside_existing_new_directory() {
        let temp = tempfile::tempdir().unwrap();
        let legacy = temp.path().join(LEGACY_APP_DIR_NAME);
        let new_dir = temp.path().join(APP_DIR_NAME);
        write_file(&new_dir.join(LEGACY_DB_FILE_NAME), b"db");
        write_file(&new_dir.join(format!("{LEGACY_DB_FILE_NAME}-wal")), b"wal");

        let resolved = resolve_app_dir(&new_dir, &legacy).unwrap();

        assert_eq!(resolved, new_dir);
        assert_eq!(fs::read(new_dir.join(DB_FILE_NAME)).unwrap(), b"db");
        assert_eq!(
            fs::read(new_dir.join(format!("{DB_FILE_NAME}-wal"))).unwrap(),
            b"wal"
        );
        assert!(!new_dir.join(LEGACY_DB_FILE_NAME).exists());
        assert!(!new_dir.join(format!("{LEGACY_DB_FILE_NAME}-wal")).exists());
    }

    #[test]
    fn falls_back_to_the_legacy_directory_when_copy_fails() {
        let temp = tempfile::tempdir().unwrap();
        let legacy = temp.path().join(LEGACY_APP_DIR_NAME);
        write_file(&legacy.join(LEGACY_DB_FILE_NAME), b"legacy-db");
        // `blocker` 是文件而不是目录：新目录位于其下层，create_dir_all 必然失败。
        let blocker = temp.path().join("blocker");
        fs::write(&blocker, b"x").unwrap();
        let new_dir = blocker.join(APP_DIR_NAME);

        let resolved = resolve_app_dir(&new_dir, &legacy).unwrap();

        assert_eq!(resolved, legacy);
        assert!(legacy.join(LEGACY_DB_FILE_NAME).exists());
    }

    #[test]
    fn db_path_uses_the_legacy_file_name_until_it_is_renamed() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("data");
        write_file(&dir.join(LEGACY_DB_FILE_NAME), b"db");
        assert_eq!(resolve_db_file_name(&dir), LEGACY_DB_FILE_NAME);
        write_file(&dir.join(DB_FILE_NAME), b"db2");
        assert_eq!(resolve_db_file_name(&dir), DB_FILE_NAME);
        let empty = temp.path().join("empty");
        fs::create_dir_all(&empty).unwrap();
        assert_eq!(resolve_db_file_name(&empty), DB_FILE_NAME);
    }
}
