use crate::domain::{TargetBinding, TargetKind};
use crate::error::{AppError, AppResult};
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashMap;

/// 把一行 `target_bindings` 映射成结构体。
///
/// 返回 `Ok(None)` 表示「这行的目标本机不认识」。**绝不冒充 Claude Code**：
/// 一旦冒充，下游会拿 Claude Code 的清理路径去删用户 `~/.claude/settings.json` 里
/// 真实的鉴权环境变量（0.1.5 移除 ZCode 目标时暴露出的事故放大器，见 design.md §3.3）。
/// 未知目标行由读取层跳过、由 `zcode_retirement::ensure_in_db`（自家退役目标）或
/// 用户手动处置其它来源的行。
fn map_binding(row: &rusqlite::Row<'_>) -> rusqlite::Result<Option<TargetBinding>> {
    let target_s: String = row.get(0)?;
    let Some(target) = TargetKind::parse(&target_s) else {
        tracing::warn!(
            target = %target_s,
            "target_bindings 出现本机不认识的目标，跳过该行（不当成 Claude Code）"
        );
        return Ok(None);
    };
    let managed_paths: String = row.get(6)?;
    let managed_env: String = row.get(7)?;
    let expected: String = row.get(8)?;
    Ok(Some(TargetBinding {
        target,
        site_id: row.get(1)?,
        site_name_snapshot: row.get(2)?,
        model_id: row.get(3)?,
        provider_id: row.get(4)?,
        key_fingerprint: row.get(5)?,
        managed_paths: serde_json::from_str(&managed_paths).unwrap_or_default(),
        managed_env_keys: serde_json::from_str(&managed_env).unwrap_or_default(),
        expected_fields: serde_json::from_str(&expected).unwrap_or_default(),
        orphan: row.get::<_, i64>(9)? != 0,
        apply_record_id: row.get(10)?,
        applied_at: row.get(11)?,
        api_key: crate::domain::BindingApiKeySnapshot {
            id: row.get(12).ok().flatten(),
            label: row.get(13).ok().flatten(),
            prefix: row.get(14).ok().flatten(),
        },
    }))
}

pub fn get_binding(conn: &Connection, target: TargetKind) -> AppResult<Option<TargetBinding>> {
    let mut stmt = conn.prepare(
        "SELECT target, site_id, site_name_snapshot, model_id, provider_id, key_fingerprint, managed_paths_json, managed_env_keys_json, expected_fields_json, orphan, apply_record_id, applied_at, site_api_key_id, site_api_key_label_snapshot, site_api_key_prefix_snapshot FROM target_bindings WHERE target = ?1",
    )?;
    Ok(stmt
        .query_row(params![target.as_str()], map_binding)
        .optional()?
        .flatten())
}

pub fn list_bindings(conn: &Connection) -> AppResult<Vec<TargetBinding>> {
    let mut stmt = conn.prepare(
        "SELECT target, site_id, site_name_snapshot, model_id, provider_id, key_fingerprint, managed_paths_json, managed_env_keys_json, expected_fields_json, orphan, apply_record_id, applied_at, site_api_key_id, site_api_key_label_snapshot, site_api_key_prefix_snapshot FROM target_bindings",
    )?;
    let rows = stmt.query_map([], map_binding)?;
    let mut out = Vec::new();
    for r in rows {
        if let Some(binding) = r? {
            out.push(binding);
        }
    }
    Ok(out)
}

pub fn upsert_binding(conn: &Connection, b: &TargetBinding) -> AppResult<()> {
    conn.execute(
        "INSERT INTO target_bindings (target, site_id, site_name_snapshot, model_id, provider_id, key_fingerprint, managed_paths_json, managed_env_keys_json, expected_fields_json, orphan, apply_record_id, applied_at, site_api_key_id, site_api_key_label_snapshot, site_api_key_prefix_snapshot)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)
         ON CONFLICT(target) DO UPDATE SET
           site_id=excluded.site_id,
           site_name_snapshot=excluded.site_name_snapshot,
           model_id=excluded.model_id,
           provider_id=excluded.provider_id,
           key_fingerprint=excluded.key_fingerprint,
           managed_paths_json=excluded.managed_paths_json,
           managed_env_keys_json=excluded.managed_env_keys_json,
           expected_fields_json=excluded.expected_fields_json,
           orphan=excluded.orphan,
           apply_record_id=excluded.apply_record_id,
           applied_at=excluded.applied_at,
           site_api_key_id=excluded.site_api_key_id,
           site_api_key_label_snapshot=excluded.site_api_key_label_snapshot,
           site_api_key_prefix_snapshot=excluded.site_api_key_prefix_snapshot",
        params![
            b.target.as_str(),
            b.site_id,
            b.site_name_snapshot,
            b.model_id,
            b.provider_id,
            b.key_fingerprint,
            serde_json::to_string(&b.managed_paths)?,
            serde_json::to_string(&b.managed_env_keys)?,
            serde_json::to_string(&b.expected_fields)?,
            b.orphan as i64,
            b.apply_record_id,
            b.applied_at,
            b.api_key.id,
            b.api_key.label,
            b.api_key.prefix
        ],
    )?;
    Ok(())
}

pub fn orphan_bindings_for_site(conn: &Connection, site_id: &str) -> AppResult<()> {
    conn.execute(
        "UPDATE target_bindings SET site_id = NULL, orphan = 1 WHERE site_id = ?1",
        params![site_id],
    )?;
    Ok(())
}

pub fn delete_binding(conn: &Connection, target: TargetKind) -> AppResult<()> {
    conn.execute(
        "DELETE FROM target_bindings WHERE target = ?1",
        params![target.as_str()],
    )?;
    Ok(())
}

pub fn list_bindings_for_site(conn: &Connection, site_id: &str) -> AppResult<Vec<TargetBinding>> {
    let mut stmt = conn.prepare(
        "SELECT target, site_id, site_name_snapshot, model_id, provider_id, key_fingerprint, managed_paths_json, managed_env_keys_json, expected_fields_json, orphan, apply_record_id, applied_at, site_api_key_id, site_api_key_label_snapshot, site_api_key_prefix_snapshot FROM target_bindings WHERE site_id = ?1",
    )?;
    let rows = stmt.query_map(params![site_id], map_binding)?;
    let mut out = Vec::new();
    for r in rows {
        if let Some(binding) = r? {
            out.push(binding);
        }
    }
    Ok(out)
}

pub fn empty_expected() -> HashMap<String, String> {
    HashMap::new()
}

pub fn binding_not_found() -> AppError {
    AppError::new("not_found", "binding not found")
}
