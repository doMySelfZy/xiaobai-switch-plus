use super::{
    column_exists, ensure_column, set_user_version, table_exists, user_version, SCHEMA_VERSION,
};
use crate::crypto::Crypto;
use crate::error::{AppError, AppResult};
use crate::paths::{app_backups_dir, master_key_path};
use rusqlite::{params, Connection};
use std::fs;
use std::path::Path;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackupMode {
    Auto,
    Skip,
    Required,
}

const V1_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS mcp_servers (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  kind TEXT NOT NULL DEFAULT 'stdio',
  enabled INTEGER NOT NULL DEFAULT 1,
  targets_json TEXT NOT NULL,
  config_json TEXT NOT NULL,
  secrets_encrypted TEXT,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_mcp_servers_updated ON mcp_servers(updated_at);

CREATE TABLE IF NOT EXISTS agent_rules (
  id INTEGER PRIMARY KEY CHECK (id = 1),
  body TEXT NOT NULL DEFAULT '',
  targets_json TEXT NOT NULL DEFAULT '[]',
  updated_at INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS settings (
  id INTEGER PRIMARY KEY CHECK (id = 1),
  json TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS sites (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  base_url TEXT NOT NULL,
  protocol TEXT NOT NULL DEFAULT 'openai_compatible',
  claude_auth_key_style TEXT NOT NULL DEFAULT 'anthropic_auth_token',
  notes TEXT,
  enabled INTEGER NOT NULL DEFAULT 1,
  sort_order INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  base_urls_json TEXT,
  capabilities_json TEXT,
  newapi_access_token_encrypted TEXT,
  newapi_user_id TEXT,
  zcode_api_type TEXT,
  proxy_headers_encrypted TEXT,
  proxy_header_count INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS site_api_keys (
  id TEXT PRIMARY KEY,
  site_id TEXT NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
  label TEXT NOT NULL,
  api_key_encrypted TEXT NOT NULL,
  key_prefix TEXT NOT NULL,
  is_active INTEGER NOT NULL DEFAULT 0,
  selected_model_id TEXT,
  last_model_fetch_at INTEGER,
  last_model_fetch_latency_ms INTEGER,
  last_model_fetch_error TEXT,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_site_api_keys_one_active
  ON site_api_keys(site_id) WHERE is_active = 1;
CREATE UNIQUE INDEX IF NOT EXISTS idx_site_api_keys_label
  ON site_api_keys(site_id, lower(label));

CREATE TABLE IF NOT EXISTS site_models (
  id TEXT PRIMARY KEY,
  site_id TEXT NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
  api_key_id TEXT NOT NULL REFERENCES site_api_keys(id) ON DELETE CASCADE,
  model_id TEXT NOT NULL,
  display_name TEXT NOT NULL,
  owned_by TEXT,
  raw_json TEXT,
  is_manual INTEGER NOT NULL DEFAULT 0,
  UNIQUE(api_key_id, model_id)
);

CREATE TABLE IF NOT EXISTS site_model_exclusions (
  site_id TEXT NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
  api_key_id TEXT NOT NULL REFERENCES site_api_keys(id) ON DELETE CASCADE,
  model_id TEXT NOT NULL,
  PRIMARY KEY (api_key_id, model_id)
);

CREATE TABLE IF NOT EXISTS site_thinking_presets (
  site_id TEXT NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
  target TEXT NOT NULL,
  json TEXT NOT NULL,
  updated_at INTEGER NOT NULL,
  PRIMARY KEY (site_id, target)
);

CREATE TABLE IF NOT EXISTS target_bindings (
  target TEXT PRIMARY KEY,
  site_id TEXT,
  site_name_snapshot TEXT NOT NULL,
  model_id TEXT NOT NULL,
  provider_id TEXT,
  key_fingerprint TEXT NOT NULL,
  managed_paths_json TEXT NOT NULL,
  managed_env_keys_json TEXT NOT NULL,
  expected_fields_json TEXT NOT NULL,
  orphan INTEGER NOT NULL DEFAULT 0,
  apply_record_id TEXT,
  applied_at INTEGER NOT NULL,
  site_api_key_id TEXT,
  site_api_key_label_snapshot TEXT,
  site_api_key_prefix_snapshot TEXT,
  FOREIGN KEY (site_id) REFERENCES sites(id) ON DELETE SET NULL,
  FOREIGN KEY (site_api_key_id) REFERENCES site_api_keys(id) ON DELETE SET NULL
);

CREATE TABLE IF NOT EXISTS apply_records (
  id TEXT PRIMARY KEY,
  site_id TEXT,
  site_name_snapshot TEXT NOT NULL,
  target TEXT NOT NULL,
  model_id TEXT NOT NULL,
  provider_id TEXT,
  status TEXT NOT NULL,
  backup_dir TEXT,
  touched_keys_json TEXT NOT NULL,
  config_snapshot_hash TEXT,
  error TEXT,
  applied_at INTEGER NOT NULL,
  site_api_key_id TEXT,
  site_api_key_label_snapshot TEXT,
  site_api_key_prefix_snapshot TEXT,
  FOREIGN KEY (site_id) REFERENCES sites(id) ON DELETE SET NULL,
  FOREIGN KEY (site_api_key_id) REFERENCES site_api_keys(id) ON DELETE SET NULL
);

CREATE TABLE IF NOT EXISTS webdav_config (
  id INTEGER PRIMARY KEY CHECK (id = 1),
  base_url TEXT NOT NULL,
  username TEXT NOT NULL,
  password_encrypted TEXT NOT NULL,
  -- 远端目录名是跨机同步的内部协议路径，与 manifest 文件名绑定：保持旧值以兼容既有机器。
  remote_path TEXT NOT NULL DEFAULT 'xiaobai-switch',
  accept_invalid_certs INTEGER NOT NULL DEFAULT 0,
  auto_sync_enabled INTEGER NOT NULL DEFAULT 0,
  sync_interval_minutes INTEGER NOT NULL DEFAULT 60,
  max_remote_backups INTEGER NOT NULL DEFAULT 3,
  updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS webdav_sync_state (
  id INTEGER PRIMARY KEY CHECK (id = 1),
  last_attempt_at INTEGER,
  last_success_at INTEGER,
  status TEXT NOT NULL DEFAULT 'never',
  error TEXT
);

CREATE TABLE IF NOT EXISTS sync_meta (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL,
  updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_apply_records_target_time ON apply_records(target, applied_at DESC);
CREATE INDEX IF NOT EXISTS idx_target_bindings_orphan ON target_bindings(orphan);
CREATE INDEX IF NOT EXISTS idx_site_api_keys_site ON site_api_keys(site_id);
CREATE INDEX IF NOT EXISTS idx_site_models_key ON site_models(api_key_id);
"#;

/// 存量库升级时 CREATE TABLE IF NOT EXISTS 不会补新列，必须显式 ALTER。
fn ensure_sites_newapi_columns(conn: &Connection) -> AppResult<()> {
    if !table_exists(conn, "sites")? {
        return Ok(());
    }
    ensure_column(
        conn,
        "sites",
        "newapi_access_token_encrypted",
        "ALTER TABLE sites ADD COLUMN newapi_access_token_encrypted TEXT",
    )?;
    ensure_column(
        conn,
        "sites",
        "newapi_user_id",
        "ALTER TABLE sites ADD COLUMN newapi_user_id TEXT",
    )?;
    // ZCode 目标用的协议（anthropic-messages / openai-responses / openai-chat-completions）。
    // 只加列、不改 SCHEMA_VERSION：版本号达标的库会走早退分支，也会跑到这里。
    ensure_column(
        conn,
        "sites",
        "zcode_api_type",
        "ALTER TABLE sites ADD COLUMN zcode_api_type TEXT",
    )?;
    Ok(())
}

fn ensure_mcp_schema(conn: &Connection) -> AppResult<()> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS mcp_servers (id TEXT PRIMARY KEY, name TEXT NOT NULL, kind TEXT NOT NULL DEFAULT 'stdio', enabled INTEGER NOT NULL DEFAULT 1, targets_json TEXT NOT NULL, config_json TEXT NOT NULL, secrets_encrypted TEXT, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL); CREATE INDEX IF NOT EXISTS idx_mcp_servers_updated ON mcp_servers(updated_at);")?;
    // 版本追踪字段
    ensure_column(
        conn,
        "mcp_servers",
        "current_version",
        "ALTER TABLE mcp_servers ADD COLUMN current_version TEXT",
    )?;
    ensure_column(
        conn,
        "mcp_servers",
        "latest_version",
        "ALTER TABLE mcp_servers ADD COLUMN latest_version TEXT",
    )?;
    ensure_column(
        conn,
        "mcp_servers",
        "last_update_check_at",
        "ALTER TABLE mcp_servers ADD COLUMN last_update_check_at INTEGER",
    )?;
    Ok(())
}

fn ensure_agent_rules_schema(conn: &Connection) -> AppResult<()> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS agent_rules (id INTEGER PRIMARY KEY CHECK (id = 1), body TEXT NOT NULL DEFAULT '', targets_json TEXT NOT NULL DEFAULT '[]', updated_at INTEGER NOT NULL DEFAULT 0);")?;
    Ok(())
}

fn ensure_agent_update_status_schema(conn: &Connection) -> AppResult<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS agent_update_status (
            kind TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            current_version TEXT,
            latest_version TEXT,
            has_update INTEGER NOT NULL DEFAULT 0,
            last_check_at INTEGER
        );"
    )?;
    Ok(())
}

/// 本地代理请求头：密文列 + 明文计数列。
fn ensure_sites_proxy_header_columns(conn: &Connection) -> AppResult<()> {
    if !table_exists(conn, "sites")? {
        return Ok(());
    }
    ensure_column(
        conn,
        "sites",
        "proxy_headers_encrypted",
        "ALTER TABLE sites ADD COLUMN proxy_headers_encrypted TEXT",
    )?;
    ensure_column(
        conn,
        "sites",
        "proxy_header_count",
        "ALTER TABLE sites ADD COLUMN proxy_header_count INTEGER NOT NULL DEFAULT 0",
    )?;
    Ok(())
}

/// 版本号之前的存量库增量补齐。`CREATE TABLE IF NOT EXISTS` 对已存在的表是空操作，
/// 所以每一个「库已存在」的分支都必须走这里，否则升版本号会让老库永远拿不到新列/新表。
fn ensure_incremental_schema(conn: &Connection) -> AppResult<()> {
    ensure_sites_newapi_columns(conn)?;
    ensure_sites_proxy_header_columns(conn)?;
    ensure_mcp_schema(conn)?;
    ensure_agent_rules_schema(conn)?;
    ensure_agent_update_status_schema(conn)?;
    // ZCode 目标撤退的数据清洗（见 zcode_retirement.rs 文件头）。刻意放在最后：
    // 它依赖上面的增量补齐把表/列准备好。失败只降级为 warn —— 让清洗下次再试，
    // 冒泡出去会变成「数据库打不开」，用户连应用都用不了。
    match crate::zcode_retirement::ensure_in_db(conn) {
        Ok(changes) => {
            if !changes.is_empty() {
                tracing::info!(
                    touched = changes.touched.len(),
                    skipped = changes.warnings.len(),
                    "ZCode 撤退清洗已落库（升级后的一次性行为）"
                );
            }
        }
        Err(error) => {
            tracing::warn!(error = %error, "ZCode 撤退清洗未完成，下次启动重试");
        }
    }
    Ok(())
}

pub fn apply_schema(
    conn: &Connection,
    crypto: Option<&Crypto>,
    backup: BackupMode,
) -> AppResult<()> {
    let version = user_version(conn)?;
    if version >= SCHEMA_VERSION {
        conn.execute_batch(V1_SCHEMA)?;
        ensure_incremental_schema(conn)?;
        return Ok(());
    }

    if needs_legacy_migration(conn)? {
        let owned;
        let crypto = match crypto {
            Some(c) => c,
            None => {
                owned = Crypto::ensure_can_decrypt_db(true)?;
                &owned
            }
        };
        migrate_legacy(conn, crypto, backup)?;
        ensure_incremental_schema(conn)?;
        return Ok(());
    }

    conn.execute_batch(V1_SCHEMA)?;
    backfill_base_urls(conn)?;
    ensure_incremental_schema(conn)?;
    set_user_version(conn, SCHEMA_VERSION)?;
    Ok(())
}

fn needs_legacy_migration(conn: &Connection) -> AppResult<bool> {
    if !table_exists(conn, "sites")? {
        return Ok(false);
    }
    if table_exists(conn, "site_api_keys")? {
        return Ok(false);
    }
    column_exists(conn, "sites", "api_key_encrypted")
}

fn migrate_legacy(conn: &Connection, crypto: &Crypto, backup: BackupMode) -> AppResult<()> {
    verify_encrypted_payloads(conn, crypto)?;
    maybe_backup(conn, backup)?;

    conn.execute_batch("PRAGMA foreign_keys = OFF;")?;
    conn.execute_batch("BEGIN IMMEDIATE;")?;
    let result = run_legacy_migration(conn);
    match result {
        Ok(()) => conn.execute_batch("COMMIT;")?,
        Err(_) => {
            let _ = conn.execute_batch("ROLLBACK;");
        }
    }
    conn.execute_batch("PRAGMA foreign_keys = ON;")?;
    result?;

    let mut stmt = conn.prepare("PRAGMA foreign_key_check")?;
    let violations: Vec<String> = stmt
        .query_map([], |row| {
            Ok(format!(
                "{}:{}",
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    if !violations.is_empty() {
        return Err(AppError::new(
            "internal",
            format!(
                "foreign key check failed after migration: {}",
                violations.join(",")
            ),
        ));
    }
    Ok(())
}

fn run_legacy_migration(conn: &Connection) -> AppResult<()> {
    conn.execute_batch(
        r#"
CREATE TABLE site_api_keys (
  id TEXT PRIMARY KEY,
  site_id TEXT NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
  label TEXT NOT NULL,
  api_key_encrypted TEXT NOT NULL,
  key_prefix TEXT NOT NULL,
  is_active INTEGER NOT NULL DEFAULT 0,
  selected_model_id TEXT,
  last_model_fetch_at INTEGER,
  last_model_fetch_latency_ms INTEGER,
  last_model_fetch_error TEXT,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
"#,
    )?;

    let mut sites = conn.prepare(
        "SELECT id, api_key_encrypted, key_prefix, selected_model_id, last_model_fetch_at, last_model_fetch_latency_ms, last_model_fetch_error, created_at, updated_at FROM sites",
    )?;
    let rows = sites.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, Option<i64>>(4)?,
            row.get::<_, Option<i64>>(5)?,
            row.get::<_, Option<String>>(6)?,
            row.get::<_, i64>(7)?,
            row.get::<_, i64>(8)?,
        ))
    })?;
    let mut site_keys = Vec::new();
    for row in rows {
        site_keys.push(row?);
    }
    drop(sites);

    let mut key_by_site = std::collections::HashMap::new();
    for (site_id, enc, prefix, selected, fetch_at, latency, fetch_err, created, updated) in
        site_keys
    {
        let key_id = Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO site_api_keys (id, site_id, label, api_key_encrypted, key_prefix, is_active, selected_model_id, last_model_fetch_at, last_model_fetch_latency_ms, last_model_fetch_error, created_at, updated_at)
             VALUES (?1,?2,'K 1',?3,?4,1,?5,?6,?7,?8,?9,?10)",
            params![
                key_id,
                site_id,
                enc,
                prefix,
                selected,
                fetch_at,
                latency,
                fetch_err,
                created,
                updated
            ],
        )?;
        key_by_site.insert(site_id, key_id);
    }

    conn.execute_batch(
        r#"
CREATE TABLE site_models_new (
  id TEXT PRIMARY KEY,
  site_id TEXT NOT NULL,
  api_key_id TEXT NOT NULL,
  model_id TEXT NOT NULL,
  display_name TEXT NOT NULL,
  owned_by TEXT,
  raw_json TEXT,
  is_manual INTEGER NOT NULL DEFAULT 0,
  UNIQUE(api_key_id, model_id)
);
CREATE TABLE site_model_exclusions_new (
  site_id TEXT NOT NULL,
  api_key_id TEXT NOT NULL,
  model_id TEXT NOT NULL,
  PRIMARY KEY (api_key_id, model_id)
);
"#,
    )?;

    if table_exists(conn, "site_models")? {
        let mut stmt = conn.prepare(
            "SELECT id, site_id, model_id, display_name, owned_by, raw_json, is_manual FROM site_models",
        )?;
        let models = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, i64>(6).unwrap_or(0),
            ))
        })?;
        for model in models {
            let (id, site_id, model_id, display, owned, raw, manual) = model?;
            let Some(key_id) = key_by_site.get(&site_id) else {
                continue;
            };
            conn.execute(
                "INSERT OR IGNORE INTO site_models_new (id, site_id, api_key_id, model_id, display_name, owned_by, raw_json, is_manual) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                params![id, site_id, key_id, model_id, display, owned, raw, manual],
            )?;
        }
        drop(stmt);
        conn.execute_batch("DROP TABLE site_models;")?;
    }
    conn.execute_batch(
        r#"
CREATE TABLE site_models (
  id TEXT PRIMARY KEY,
  site_id TEXT NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
  api_key_id TEXT NOT NULL REFERENCES site_api_keys(id) ON DELETE CASCADE,
  model_id TEXT NOT NULL,
  display_name TEXT NOT NULL,
  owned_by TEXT,
  raw_json TEXT,
  is_manual INTEGER NOT NULL DEFAULT 0,
  UNIQUE(api_key_id, model_id)
);
INSERT INTO site_models SELECT * FROM site_models_new;
DROP TABLE site_models_new;
"#,
    )?;

    if table_exists(conn, "site_model_exclusions")? {
        let mut stmt = conn.prepare("SELECT site_id, model_id FROM site_model_exclusions")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (site_id, model_id) = row?;
            let Some(key_id) = key_by_site.get(&site_id) else {
                continue;
            };
            conn.execute(
                "INSERT OR IGNORE INTO site_model_exclusions_new (site_id, api_key_id, model_id) VALUES (?1,?2,?3)",
                params![site_id, key_id, model_id],
            )?;
        }
        drop(stmt);
        conn.execute_batch("DROP TABLE site_model_exclusions;")?;
    }
    conn.execute_batch(
        r#"
CREATE TABLE site_model_exclusions (
  site_id TEXT NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
  api_key_id TEXT NOT NULL REFERENCES site_api_keys(id) ON DELETE CASCADE,
  model_id TEXT NOT NULL,
  PRIMARY KEY (api_key_id, model_id)
);
INSERT INTO site_model_exclusions SELECT * FROM site_model_exclusions_new;
DROP TABLE site_model_exclusions_new;
"#,
    )?;

    if table_exists(conn, "sites")? {
        ensure_sites_newapi_columns(conn)?;
    }

    if table_exists(conn, "target_bindings")? {
        ensure_column(
            conn,
            "target_bindings",
            "site_api_key_id",
            "ALTER TABLE target_bindings ADD COLUMN site_api_key_id TEXT",
        )?;
        ensure_column(
            conn,
            "target_bindings",
            "site_api_key_label_snapshot",
            "ALTER TABLE target_bindings ADD COLUMN site_api_key_label_snapshot TEXT",
        )?;
        ensure_column(
            conn,
            "target_bindings",
            "site_api_key_prefix_snapshot",
            "ALTER TABLE target_bindings ADD COLUMN site_api_key_prefix_snapshot TEXT",
        )?;
        conn.execute(
            "UPDATE target_bindings SET site_api_key_id = (SELECT id FROM site_api_keys WHERE site_api_keys.site_id = target_bindings.site_id AND is_active = 1), site_api_key_label_snapshot = 'K 1', site_api_key_prefix_snapshot = (SELECT key_prefix FROM site_api_keys WHERE site_api_keys.site_id = target_bindings.site_id AND is_active = 1) WHERE site_id IS NOT NULL",
            [],
        )?;
    }

    if table_exists(conn, "apply_records")? {
        ensure_column(
            conn,
            "apply_records",
            "site_api_key_id",
            "ALTER TABLE apply_records ADD COLUMN site_api_key_id TEXT",
        )?;
        ensure_column(
            conn,
            "apply_records",
            "site_api_key_label_snapshot",
            "ALTER TABLE apply_records ADD COLUMN site_api_key_label_snapshot TEXT",
        )?;
        ensure_column(
            conn,
            "apply_records",
            "site_api_key_prefix_snapshot",
            "ALTER TABLE apply_records ADD COLUMN site_api_key_prefix_snapshot TEXT",
        )?;
        conn.execute(
            "UPDATE apply_records SET site_api_key_id = (SELECT id FROM site_api_keys WHERE site_api_keys.site_id = apply_records.site_id AND is_active = 1), site_api_key_label_snapshot = 'K 1', site_api_key_prefix_snapshot = (SELECT key_prefix FROM site_api_keys WHERE site_api_keys.site_id = apply_records.site_id AND is_active = 1) WHERE site_id IS NOT NULL",
            [],
        )?;
    }

    conn.execute_batch(
        r#"
CREATE TABLE sites_new (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  base_url TEXT NOT NULL,
  protocol TEXT NOT NULL DEFAULT 'openai_compatible',
  claude_auth_key_style TEXT NOT NULL DEFAULT 'anthropic_auth_token',
  notes TEXT,
  enabled INTEGER NOT NULL DEFAULT 1,
  sort_order INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  base_urls_json TEXT,
  capabilities_json TEXT
);
INSERT INTO sites_new (id, name, base_url, protocol, claude_auth_key_style, notes, enabled, sort_order, created_at, updated_at, base_urls_json, capabilities_json)
SELECT id, name, base_url, protocol, claude_auth_key_style, notes, enabled, sort_order, created_at, updated_at, base_urls_json, capabilities_json FROM sites;
DROP TABLE sites;
ALTER TABLE sites_new RENAME TO sites;
"#,
    )?;

    conn.execute_batch(V1_SCHEMA)?;

    let sites: i64 = conn.query_row("SELECT COUNT(*) FROM sites", [], |r| r.get(0))?;
    let keys: i64 = conn.query_row("SELECT COUNT(*) FROM site_api_keys", [], |r| r.get(0))?;
    if sites != keys {
        return Err(AppError::new(
            "internal",
            format!("migration key count mismatch: sites={sites} keys={keys}"),
        ));
    }
    let active_sites: i64 = conn.query_row(
        "SELECT COUNT(*) FROM (SELECT site_id FROM site_api_keys WHERE is_active = 1 GROUP BY site_id)",
        [],
        |r| r.get(0),
    )?;
    if active_sites != sites {
        return Err(AppError::new(
            "internal",
            "migration did not produce exactly one active key per site",
        ));
    }

    set_user_version(conn, SCHEMA_VERSION)?;
    Ok(())
}

pub fn verify_encrypted_payloads(conn: &Connection, crypto: &Crypto) -> AppResult<()> {
    let mut payloads = Vec::new();
    if table_exists(conn, "sites")? && column_exists(conn, "sites", "api_key_encrypted")? {
        let mut stmt = conn.prepare("SELECT api_key_encrypted FROM sites")?;
        for value in stmt.query_map([], |row| row.get::<_, String>(0))? {
            payloads.push(value?);
        }
    }
    if table_exists(conn, "site_api_keys")? {
        let mut stmt = conn.prepare("SELECT api_key_encrypted FROM site_api_keys")?;
        for value in stmt.query_map([], |row| row.get::<_, String>(0))? {
            payloads.push(value?);
        }
    }
    if table_exists(conn, "webdav_config")? {
        let mut stmt = conn.prepare("SELECT password_encrypted FROM webdav_config")?;
        for value in stmt.query_map([], |row| row.get::<_, String>(0))? {
            payloads.push(value?);
        }
    }
    for value in payloads {
        if value.trim().is_empty() {
            continue;
        }
        crypto.decrypt(&value).map_err(|_| {
            AppError::new(
                "master_key_missing",
                "stored ciphertext cannot be decrypted with the current master.key",
            )
        })?;
    }
    Ok(())
}

fn maybe_backup(conn: &Connection, mode: BackupMode) -> AppResult<()> {
    if mode == BackupMode::Skip {
        return Ok(());
    }
    let path = conn.path().map(Path::new);
    let is_file = path.is_some_and(|p| p.exists() && p.file_name().is_some());
    if !is_file {
        if mode == BackupMode::Required {
            return Err(AppError::new(
                "backup_failed",
                "cannot create pre-migration backup for an in-memory database",
            ));
        }
        return Ok(());
    }
    backup_pre_migration(conn)
}

fn backup_pre_migration(conn: &Connection) -> AppResult<()> {
    backup_snapshot(conn, "pre_migration_site_api_keys", true, true)?;
    Ok(())
}

/// 整库快照的共用实现：`VACUUM INTO` 一份数据库 + 随行的 `master.key`。
///
/// 两个调用方的诉求正好相反，所以做成参数而不是复制一份易错的备份逻辑：
/// - 迁移前快照（`backup_pre_migration`）：每次跑迁移都重拍，且**必须**拿到 master.key，
///   否则快照解不开密文，等于没有；
/// - ZCode 撤退快照（`zcode_retirement::backup_before_first_write`）：只留**最早**那一份
///   （要的是「清洗前」的状态，后续轮次数据已被改动，重拍无意义），并且缺 key 只 warn：
///   清洗推迟到下次启动无所谓，让 `Db::open` 失败会让用户连应用都打不开。
///
/// 返回值 = 这次是否真的写了快照（`false` = 已存在同名额的快照，按策略保留旧的）。
pub(crate) fn backup_snapshot(
    conn: &Connection,
    subdir: &str,
    replace_existing: bool,
    require_master_key: bool,
) -> AppResult<bool> {
    let dest_dir = app_backups_dir()?.join(subdir);
    fs::create_dir_all(&dest_dir)?;
    let dest_db = dest_dir.join("xiaobai-switch.db");
    if dest_db.exists() {
        if !replace_existing {
            return Ok(false);
        }
        fs::remove_file(&dest_db)?;
    }
    let escaped = dest_db.to_string_lossy().replace('\'', "''");
    conn.execute_batch(&format!("VACUUM INTO '{escaped}'"))
        .map_err(|e| {
            AppError::new(
                "backup_failed",
                format!("pre-migration database backup failed: {e}"),
            )
        })?;
    let key = master_key_path()?;
    if key.exists() {
        fs::copy(&key, dest_dir.join("master.key")).map_err(|e| {
            AppError::new(
                "backup_failed",
                format!("pre-migration master.key backup failed: {e}"),
            )
        })?;
    } else if require_master_key {
        return Err(AppError::new(
            "master_key_missing",
            "Cannot decrypt stored API keys: master.key is missing",
        ));
    } else {
        tracing::warn!(
            dir = %dest_dir.display(),
            "master.key 缺失，整库快照未含解密密钥"
        );
    }
    Ok(true)
}

fn backfill_base_urls(conn: &Connection) -> AppResult<()> {
    if !table_exists(conn, "sites")? {
        return Ok(());
    }
    let mut stmt = conn.prepare("SELECT id, base_url, base_urls_json FROM sites")?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?,
        ))
    })?;
    let mut updates = Vec::new();
    for r in rows {
        let (id, base_url, json) = r?;
        let needs = match json.as_deref() {
            None | Some("") => true,
            Some(s) => serde_json::from_str::<Vec<String>>(s)
                .ok()
                .filter(|v| !v.is_empty())
                .is_none(),
        };
        if needs {
            let encoded = serde_json::to_string(&vec![base_url])?;
            updates.push((id, encoded));
        }
    }
    drop(stmt);
    for (id, json) in updates {
        conn.execute(
            "UPDATE sites SET base_urls_json = ?2 WHERE id = ?1",
            rusqlite::params![id, json],
        )?;
    }
    Ok(())
}

#[cfg(test)]
pub fn install_legacy_schema(conn: &Connection) -> AppResult<()> {
    conn.execute_batch(
        r#"
PRAGMA foreign_keys = ON;
CREATE TABLE settings (
  id INTEGER PRIMARY KEY CHECK (id = 1),
  json TEXT NOT NULL
);
CREATE TABLE sites (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  base_url TEXT NOT NULL,
  api_key_encrypted TEXT NOT NULL,
  key_prefix TEXT NOT NULL,
  protocol TEXT NOT NULL DEFAULT 'openai_compatible',
  claude_auth_key_style TEXT NOT NULL DEFAULT 'anthropic_auth_token',
  notes TEXT,
  enabled INTEGER NOT NULL DEFAULT 1,
  sort_order INTEGER NOT NULL DEFAULT 0,
  selected_model_id TEXT,
  last_model_fetch_at INTEGER,
  last_model_fetch_latency_ms INTEGER,
  last_model_fetch_error TEXT,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  base_urls_json TEXT,
  capabilities_json TEXT
);
CREATE TABLE site_models (
  id TEXT PRIMARY KEY,
  site_id TEXT NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
  model_id TEXT NOT NULL,
  display_name TEXT NOT NULL,
  owned_by TEXT,
  raw_json TEXT,
  is_manual INTEGER NOT NULL DEFAULT 0,
  UNIQUE(site_id, model_id)
);
CREATE TABLE site_model_exclusions (
  site_id TEXT NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
  model_id TEXT NOT NULL,
  PRIMARY KEY (site_id, model_id)
);
CREATE TABLE target_bindings (
  target TEXT PRIMARY KEY,
  site_id TEXT,
  site_name_snapshot TEXT NOT NULL,
  model_id TEXT NOT NULL,
  provider_id TEXT,
  key_fingerprint TEXT NOT NULL,
  managed_paths_json TEXT NOT NULL,
  managed_env_keys_json TEXT NOT NULL,
  expected_fields_json TEXT NOT NULL,
  orphan INTEGER NOT NULL DEFAULT 0,
  apply_record_id TEXT,
  applied_at INTEGER NOT NULL,
  FOREIGN KEY (site_id) REFERENCES sites(id) ON DELETE SET NULL
);
CREATE TABLE apply_records (
  id TEXT PRIMARY KEY,
  site_id TEXT,
  site_name_snapshot TEXT NOT NULL,
  target TEXT NOT NULL,
  model_id TEXT NOT NULL,
  provider_id TEXT,
  status TEXT NOT NULL,
  backup_dir TEXT,
  touched_keys_json TEXT NOT NULL,
  config_snapshot_hash TEXT,
  error TEXT,
  applied_at INTEGER NOT NULL,
  FOREIGN KEY (site_id) REFERENCES sites(id) ON DELETE SET NULL
);
"#,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::Crypto;

    fn crypto() -> Crypto {
        Crypto::from_key([7u8; 32])
    }

    #[test]
    fn empty_db_installs_v1_without_legacy_key_columns() {
        let conn = Connection::open_in_memory().unwrap();
        apply_schema(&conn, None, BackupMode::Skip).unwrap();
        assert_eq!(user_version(&conn).unwrap(), SCHEMA_VERSION);
        assert!(table_exists(&conn, "site_api_keys").unwrap());
        assert!(table_exists(&conn, "site_thinking_presets").unwrap());
        assert!(!column_exists(&conn, "sites", "api_key_encrypted").unwrap());
    }

    #[test]
    fn fresh_database_has_local_proxy_columns() {
        let conn = Connection::open_in_memory().unwrap();
        apply_schema(&conn, None, BackupMode::Skip).unwrap();
        assert!(column_exists(&conn, "sites", "proxy_headers_encrypted").unwrap());
        assert!(column_exists(&conn, "sites", "proxy_header_count").unwrap());
    }

    #[test]
    fn remote_retention_defaults_to_three_and_never_rewrites_existing_rows() {
        // R4.1：新配置的远端保留数是 3。
        let conn = Connection::open_in_memory().unwrap();
        apply_schema(&conn, None, BackupMode::Skip).unwrap();
        conn.execute(
            "INSERT INTO webdav_config
               (id, base_url, username, password_encrypted, remote_path, updated_at)
             VALUES (1, 'https://dav.example.com/', 'alice', 'ciphertext', 'xiaobai-switch', 1)",
            [],
        )
        .unwrap();
        let default_retention: u32 = conn
            .query_row(
                "SELECT max_remote_backups FROM webdav_config WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(default_retention, 3);

        // R4.3：已存在配置的取值不被静默改写（改默认值不得演变成一次数据迁移）。
        conn.execute(
            "UPDATE webdav_config SET max_remote_backups = 10 WHERE id = 1",
            [],
        )
        .unwrap();
        apply_schema(&conn, None, BackupMode::Skip).unwrap();
        let kept: u32 = conn
            .query_row(
                "SELECT max_remote_backups FROM webdav_config WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(kept, 10);
    }

    #[test]
    fn existing_database_gains_local_proxy_columns_on_upgrade() {
        // 回归：存量库走「版本落后」分支时，CREATE TABLE IF NOT EXISTS 不会补列，
        // 必须由 ensure_incremental_schema 的 ensure_column 补齐。
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE sites (
               id TEXT PRIMARY KEY,
               name TEXT NOT NULL,
               base_url TEXT NOT NULL,
               protocol TEXT NOT NULL,
               claude_auth_key_style TEXT NOT NULL,
               notes TEXT,
               enabled INTEGER NOT NULL,
               sort_order INTEGER NOT NULL,
               created_at INTEGER NOT NULL,
               updated_at INTEGER NOT NULL,
               base_urls_json TEXT,
               capabilities_json TEXT
             );
             CREATE TABLE site_api_keys (
               id TEXT PRIMARY KEY,
               site_id TEXT NOT NULL,
               label TEXT NOT NULL,
               api_key_encrypted TEXT NOT NULL,
               key_prefix TEXT NOT NULL,
               is_active INTEGER NOT NULL DEFAULT 0,
               created_at INTEGER NOT NULL,
               updated_at INTEGER NOT NULL
             );
             CREATE TABLE settings (
               id INTEGER PRIMARY KEY CHECK (id = 1),
               json TEXT NOT NULL
             );
             PRAGMA user_version = 2;",
        )
        .unwrap();

        apply_schema(&conn, None, BackupMode::Skip).unwrap();

        assert!(column_exists(&conn, "sites", "proxy_headers_encrypted").unwrap());
        assert!(column_exists(&conn, "sites", "proxy_header_count").unwrap());
        assert_eq!(user_version(&conn).unwrap(), SCHEMA_VERSION);
    }

    #[test]
    fn repeated_apply_schema_is_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        apply_schema(&conn, None, BackupMode::Skip).unwrap();
        apply_schema(&conn, None, BackupMode::Skip).unwrap();
        assert_eq!(user_version(&conn).unwrap(), SCHEMA_VERSION);
    }

    #[test]
    fn migrates_legacy_site_key_and_models() {
        let conn = Connection::open_in_memory().unwrap();
        install_legacy_schema(&conn).unwrap();
        let crypto = crypto();
        let enc = crypto.encrypt("sk-legacy-secret").unwrap();
        conn.execute(
            "INSERT INTO sites (id, name, base_url, api_key_encrypted, key_prefix, protocol, claude_auth_key_style, notes, enabled, sort_order, selected_model_id, last_model_fetch_at, last_model_fetch_latency_ms, last_model_fetch_error, created_at, updated_at, base_urls_json)
             VALUES ('s1', 'Relay', 'https://api.example.com', ?1, 'sk-l…cret', 'openai_compatible', 'anthropic_auth_token', NULL, 1, 0, 'gpt-4.1', 9, 12, NULL, 1, 1, '[\"https://api.example.com\"]')",
            params![enc],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO site_models (id, site_id, model_id, display_name, owned_by, raw_json, is_manual) VALUES ('m1', 's1', 'gpt-4.1', 'gpt-4.1', 'openai', NULL, 0)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO site_model_exclusions (site_id, model_id) VALUES ('s1', 'hidden')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO target_bindings (target, site_id, site_name_snapshot, model_id, provider_id, key_fingerprint, managed_paths_json, managed_env_keys_json, expected_fields_json, orphan, apply_record_id, applied_at)
             VALUES ('claude_code', 's1', 'Relay', 'gpt-4.1', NULL, 'fp', '[]', '[]', '{}', 0, NULL, 1)",
            [],
        )
        .unwrap();

        apply_schema(&conn, Some(&crypto), BackupMode::Skip).unwrap();
        assert_eq!(user_version(&conn).unwrap(), SCHEMA_VERSION);
        assert!(!column_exists(&conn, "sites", "api_key_encrypted").unwrap());

        let (label, active, selected, stored): (String, i64, Option<String>, String) = conn
            .query_row(
                "SELECT label, is_active, selected_model_id, api_key_encrypted FROM site_api_keys WHERE site_id='s1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(label, "K 1");
        assert_eq!(active, 1);
        assert_eq!(selected.as_deref(), Some("gpt-4.1"));
        assert_eq!(crypto.decrypt(&stored).unwrap(), "sk-legacy-secret");

        let model_key: String = conn
            .query_row(
                "SELECT api_key_id FROM site_models WHERE model_id='gpt-4.1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let exclusion_key: String = conn
            .query_row(
                "SELECT api_key_id FROM site_model_exclusions WHERE model_id='hidden'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let binding_key: Option<String> = conn
            .query_row(
                "SELECT site_api_key_id FROM target_bindings WHERE target='claude_code'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(Some(model_key.clone()), binding_key);
        assert_eq!(model_key, exclusion_key);
    }

    #[test]
    fn migration_rejects_wrong_master_key() {
        let conn = Connection::open_in_memory().unwrap();
        install_legacy_schema(&conn).unwrap();
        let enc = crypto().encrypt("sk-legacy-secret").unwrap();
        conn.execute(
            "INSERT INTO sites (id, name, base_url, api_key_encrypted, key_prefix, protocol, claude_auth_key_style, notes, enabled, sort_order, selected_model_id, last_model_fetch_at, last_model_fetch_latency_ms, last_model_fetch_error, created_at, updated_at)
             VALUES ('s1', 'Relay', 'https://api.example.com', ?1, 'sk-xx', 'openai_compatible', 'anthropic_auth_token', NULL, 1, 0, NULL, NULL, NULL, NULL, 1, 1)",
            params![enc],
        )
        .unwrap();
        let wrong = Crypto::from_key([8u8; 32]);
        let err = apply_schema(&conn, Some(&wrong), BackupMode::Skip).unwrap_err();
        assert!(err.to_string().contains("cannot be decrypted"));
        assert_eq!(user_version(&conn).unwrap(), 0);
        assert!(!table_exists(&conn, "site_api_keys").unwrap());
    }

    #[test]
    fn existing_database_gains_newapi_columns_on_upgrade() {
        // 回归：ensure_sites_newapi_columns 必须补齐 newapi 列。
        // 测试场景：已完成 v1→v2 迁移（sites 已无 api_key_encrypted），但缺 newapi 列。
        let conn = Connection::open_in_memory().unwrap();
        
        // 创建已迁移但缺 newapi 列的 sites 表
        conn.execute_batch(
            "CREATE TABLE sites (
               id TEXT PRIMARY KEY,
               name TEXT NOT NULL,
               base_url TEXT NOT NULL,
               protocol TEXT NOT NULL,
               claude_auth_key_style TEXT NOT NULL,
               notes TEXT,
               enabled INTEGER NOT NULL,
               sort_order INTEGER NOT NULL,
               created_at INTEGER NOT NULL,
               updated_at INTEGER NOT NULL,
               base_urls_json TEXT,
               capabilities_json TEXT
             );
             CREATE TABLE site_api_keys (
               id TEXT PRIMARY KEY,
               site_id TEXT NOT NULL,
               label TEXT NOT NULL,
               api_key_encrypted TEXT NOT NULL,
               key_prefix TEXT NOT NULL,
               is_active INTEGER NOT NULL DEFAULT 0,
               created_at INTEGER NOT NULL,
               updated_at INTEGER NOT NULL
             );
             PRAGMA user_version = 2;",
        )
        .unwrap();
        
        conn.execute(
            "INSERT INTO sites (id, name, base_url, protocol, claude_auth_key_style, enabled, sort_order, created_at, updated_at)
             VALUES ('s1', 'Test', 'https://api.test', 'openai_compatible', 'anthropic_auth_token', 1, 0, 1, 1)",
            [],
        )
        .unwrap();
        
        // 确认初始状态：无 newapi 列
        let columns_before: Vec<String> = conn
            .prepare("PRAGMA table_info(sites)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert!(!columns_before.contains(&"newapi_access_token_encrypted".into()));
        
        // apply_schema 会调用 ensure_sites_newapi_columns
        apply_schema(&conn, None, BackupMode::Skip).unwrap();
        
        // 验证 newapi 列已添加
        let columns_after: Vec<String> = conn
            .prepare("PRAGMA table_info(sites)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert!(columns_after.contains(&"newapi_access_token_encrypted".into()));
        assert!(columns_after.contains(&"newapi_user_id".into()));
    }

    #[test]
    fn v1_database_reaches_current_schema_with_mcp_table() {
        // 回归：把 SCHEMA_VERSION 从 1 提到 2 时，user_version=1 的存量库会进入
        // 「版本落后」分支；该分支必须补齐 MCP 表，同时不能丢掉 newapi 列的补齐。
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE sites (
               id TEXT PRIMARY KEY,
               name TEXT NOT NULL,
               base_url TEXT NOT NULL,
               protocol TEXT NOT NULL,
               claude_auth_key_style TEXT NOT NULL,
               notes TEXT,
               enabled INTEGER NOT NULL,
               sort_order INTEGER NOT NULL,
               created_at INTEGER NOT NULL,
               updated_at INTEGER NOT NULL,
               base_urls_json TEXT,
               capabilities_json TEXT
             );
             CREATE TABLE site_api_keys (
               id TEXT PRIMARY KEY,
               site_id TEXT NOT NULL,
               label TEXT NOT NULL,
               api_key_encrypted TEXT NOT NULL,
               key_prefix TEXT NOT NULL,
               is_active INTEGER NOT NULL DEFAULT 0,
               created_at INTEGER NOT NULL,
               updated_at INTEGER NOT NULL
             );
             CREATE TABLE settings (
               id INTEGER PRIMARY KEY CHECK (id = 1),
               json TEXT NOT NULL
             );
             PRAGMA user_version = 1;",
        )
        .unwrap();

        apply_schema(&conn, None, BackupMode::Skip).unwrap();

        assert!(table_exists(&conn, "mcp_servers").unwrap());
        assert!(table_exists(&conn, "agent_rules").unwrap());
        assert!(column_exists(&conn, "sites", "newapi_access_token_encrypted").unwrap());
        assert_eq!(user_version(&conn).unwrap(), SCHEMA_VERSION);
    }

    #[test]
    fn current_version_database_still_gains_agent_rules_table() {
        // 回归：`CREATE TABLE IF NOT EXISTS` 不会给已存在的库补新表。版本号已经等于
        // 当前值时会走 `version >= SCHEMA_VERSION` 的提前返回分支，那条分支同样必须
        // 调用 ensure_incremental_schema，否则老库永远拿不到 agent_rules 且不报错。
        let conn = Connection::open_in_memory().unwrap();
        apply_schema(&conn, None, BackupMode::Skip).unwrap();
        conn.execute_batch("DROP TABLE IF EXISTS agent_rules;")
            .unwrap();

        apply_schema(&conn, None, BackupMode::Skip).unwrap();

        assert!(table_exists(&conn, "agent_rules").unwrap());
    }

    #[test]
    fn legacy_database_keeps_newapi_columns_and_gains_mcp_table() {
        // 回归：needs_legacy_migration 分支重建 sites 后，newapi 列不会由
        // CREATE TABLE IF NOT EXISTS 自动补回，必须显式补齐；MCP 表同样要建出来。
        let conn = Connection::open_in_memory().unwrap();
        install_legacy_schema(&conn).unwrap();
        let crypto = crypto();
        conn.execute(
            "INSERT INTO sites (id, name, base_url, api_key_encrypted, key_prefix, protocol, claude_auth_key_style, notes, enabled, sort_order, created_at, updated_at)
             VALUES ('s1', 'Relay', 'https://api.example.com', ?1, 'sk-…', 'openai_compatible', 'anthropic_auth_token', NULL, 1, 0, 1, 1)",
            params![crypto.encrypt("sk-legacy-secret").unwrap()],
        )
        .unwrap();

        apply_schema(&conn, Some(&crypto), BackupMode::Skip).unwrap();

        assert!(table_exists(&conn, "mcp_servers").unwrap());
        assert!(table_exists(&conn, "agent_rules").unwrap());
        assert!(column_exists(&conn, "sites", "newapi_access_token_encrypted").unwrap());
        assert!(column_exists(&conn, "sites", "newapi_user_id").unwrap());
        assert_eq!(user_version(&conn).unwrap(), SCHEMA_VERSION);
    }

}
