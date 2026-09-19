use crate::crypto::{key_fingerprint, key_prefix, Crypto};
use crate::domain::{SiteApiKeyRow, SiteApiKeySummary, UpsertSiteApiKeyInput};
use crate::error::{AppError, AppResult};
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashSet;
use uuid::Uuid;

const KEY_COLS: &str = "id, site_id, label, api_key_encrypted, key_prefix, is_active, selected_model_id, last_model_fetch_at, last_model_fetch_latency_ms, last_model_fetch_error, created_at, updated_at";

fn map_key(row: &rusqlite::Row<'_>) -> rusqlite::Result<SiteApiKeyRow> {
    Ok(SiteApiKeyRow {
        id: row.get(0)?,
        site_id: row.get(1)?,
        label: row.get(2)?,
        api_key_encrypted: row.get(3)?,
        key_prefix: row.get(4)?,
        is_active: row.get::<_, i64>(5)? != 0,
        selected_model_id: row.get(6)?,
        last_model_fetch_at: row.get(7)?,
        last_model_fetch_latency_ms: row.get(8)?,
        last_model_fetch_error: row.get(9)?,
        created_at: row.get(10)?,
        updated_at: row.get(11)?,
    })
}

pub fn list_for_site(conn: &Connection, site_id: &str) -> AppResult<Vec<SiteApiKeyRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {KEY_COLS} FROM site_api_keys WHERE site_id = ?1 ORDER BY created_at ASC, id ASC"
    ))?;
    let rows = stmt.query_map(params![site_id], map_key)?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

pub fn summaries_for_site(conn: &Connection, site_id: &str) -> AppResult<Vec<SiteApiKeySummary>> {
    Ok(list_for_site(conn, site_id)?
        .into_iter()
        .map(|row| row.to_summary())
        .collect())
}

pub fn get(conn: &Connection, id: &str) -> AppResult<SiteApiKeyRow> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {KEY_COLS} FROM site_api_keys WHERE id = ?1"
    ))?;
    stmt.query_row(params![id], map_key)
        .optional()?
        .ok_or_else(|| AppError::new("not_found", "api key not found"))
}

pub fn get_for_site(conn: &Connection, site_id: &str, id: &str) -> AppResult<SiteApiKeyRow> {
    let key = get(conn, id)?;
    if key.site_id != site_id {
        return Err(AppError::new(
            "validation_failed",
            "api key does not belong to this site",
        ));
    }
    Ok(key)
}

pub fn get_active(conn: &Connection, site_id: &str) -> AppResult<SiteApiKeyRow> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {KEY_COLS} FROM site_api_keys WHERE site_id = ?1 AND is_active = 1"
    ))?;
    stmt.query_row(params![site_id], map_key)
        .optional()?
        .ok_or_else(|| AppError::new("not_found", "active api key not found"))
}

pub fn require_active(
    conn: &Connection,
    site_id: &str,
    api_key_id: &str,
) -> AppResult<SiteApiKeyRow> {
    let key = get_for_site(conn, site_id, api_key_id)?;
    if !key.is_active {
        return Err(AppError::new(
            "validation_failed",
            "api key is not the site's current key",
        ));
    }
    Ok(key)
}

pub fn decrypt(crypto: &Crypto, key: &SiteApiKeyRow) -> AppResult<String> {
    if key.api_key_encrypted.is_empty() {
        return Ok(String::new());
    }
    crypto.decrypt(&key.api_key_encrypted)
}

pub fn next_label(conn: &Connection, site_id: &str) -> AppResult<String> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM site_api_keys WHERE site_id = ?1",
        params![site_id],
        |r| r.get(0),
    )?;
    let mut n = count + 1;
    loop {
        let label = format!("K {n}");
        if !label_taken(conn, site_id, &label, None)? {
            return Ok(label);
        }
        n += 1;
    }
}

fn normalize_label(raw: &str) -> AppResult<String> {
    let label = raw.trim();
    if label.is_empty() {
        return Err(AppError::new("validation_failed", "key name is required"));
    }
    if label.chars().count() > 64 {
        return Err(AppError::new(
            "validation_failed",
            "key name exceeds 64 characters",
        ));
    }
    Ok(label.to_string())
}

fn label_taken(
    conn: &Connection,
    site_id: &str,
    label: &str,
    except_id: Option<&str>,
) -> AppResult<bool> {
    let n: i64 = match except_id {
        Some(id) => conn.query_row(
            "SELECT COUNT(*) FROM site_api_keys WHERE site_id = ?1 AND lower(label) = lower(?2) AND id != ?3",
            params![site_id, label, id],
            |r| r.get(0),
        )?,
        None => conn.query_row(
            "SELECT COUNT(*) FROM site_api_keys WHERE site_id = ?1 AND lower(label) = lower(?2)",
            params![site_id, label],
            |r| r.get(0),
        )?,
    };
    Ok(n > 0)
}

fn assert_unique_label(
    conn: &Connection,
    site_id: &str,
    label: &str,
    except_id: Option<&str>,
) -> AppResult<()> {
    if label_taken(conn, site_id, label, except_id)? {
        return Err(AppError::new(
            "validation_failed",
            "key name already exists on this site",
        ));
    }
    Ok(())
}

fn assert_unique_secret(
    conn: &Connection,
    crypto: &Crypto,
    site_id: &str,
    secret: &str,
    except_id: Option<&str>,
) -> AppResult<()> {
    for existing in list_for_site(conn, site_id)? {
        if except_id == Some(existing.id.as_str()) {
            continue;
        }
        if decrypt(crypto, &existing)? == secret {
            return Err(AppError::new(
                "validation_failed",
                "this API key already exists on the site",
            ));
        }
    }
    Ok(())
}

pub fn insert_active(
    conn: &Connection,
    crypto: &Crypto,
    site_id: &str,
    label: &str,
    api_key: &str,
) -> AppResult<SiteApiKeyRow> {
    insert(conn, crypto, site_id, label, api_key, true)
}

pub fn insert(
    conn: &Connection,
    crypto: &Crypto,
    site_id: &str,
    label: &str,
    api_key: &str,
    active: bool,
) -> AppResult<SiteApiKeyRow> {
    let label = normalize_label(label)?;
    let secret = api_key.trim();
    if secret.is_empty() {
        return Err(AppError::new("validation_failed", "API key is required"));
    }
    assert_unique_label(conn, site_id, &label, None)?;
    assert_unique_secret(conn, crypto, site_id, secret, None)?;
    let id = Uuid::new_v4().to_string();
    let last: i64 = conn.query_row(
        "SELECT COALESCE(MAX(created_at), 0) FROM site_api_keys WHERE site_id = ?1",
        params![site_id],
        |r| r.get(0),
    )?;
    let now = Utc::now().timestamp_millis().max(last + 1);
    let enc = crypto.encrypt(secret)?;
    conn.execute(
        "INSERT INTO site_api_keys (id, site_id, label, api_key_encrypted, key_prefix, is_active, selected_model_id, last_model_fetch_at, last_model_fetch_latency_ms, last_model_fetch_error, created_at, updated_at)
         VALUES (?1,?2,?3,?4,?5,?6,NULL,NULL,NULL,NULL,?7,?7)",
        params![
            id,
            site_id,
            label,
            enc,
            key_prefix(secret),
            active as i64,
            now
        ],
    )?;
    get(conn, &id)
}

pub fn add(
    conn: &Connection,
    crypto: &Crypto,
    site_id: &str,
    label: Option<&str>,
    api_key: &str,
) -> AppResult<SiteApiKeyRow> {
    let site_exists: bool = conn
        .query_row(
            "SELECT 1 FROM sites WHERE id = ?1",
            params![site_id],
            |_| Ok(true),
        )
        .optional()?
        .unwrap_or(false);
    if !site_exists {
        return Err(AppError::new("not_found", "site not found"));
    }
    let owned = match label {
        Some(value) => normalize_label(value)?,
        None => next_label(conn, site_id)?,
    };
    insert(conn, crypto, site_id, &owned, api_key, false)
}

pub fn update(
    conn: &Connection,
    crypto: &Crypto,
    site_id: &str,
    api_key_id: &str,
    label: Option<&str>,
    api_key: Option<&str>,
) -> AppResult<SiteApiKeyRow> {
    let mut key = get_for_site(conn, site_id, api_key_id)?;
    if let Some(label) = label {
        let label = normalize_label(label)?;
        assert_unique_label(conn, site_id, &label, Some(&key.id))?;
        key.label = label;
    }
    if let Some(api_key) = api_key {
        let secret = api_key.trim();
        if secret.is_empty() {
            return Err(AppError::new("validation_failed", "API key is required"));
        }
        assert_unique_secret(conn, crypto, site_id, secret, Some(&key.id))?;
        key.api_key_encrypted = crypto.encrypt(secret)?;
        key.key_prefix = key_prefix(secret);
    }
    key.updated_at = Utc::now().timestamp_millis();
    conn.execute(
        "UPDATE site_api_keys SET label=?2, api_key_encrypted=?3, key_prefix=?4, updated_at=?5 WHERE id=?1",
        params![
            key.id,
            key.label,
            key.api_key_encrypted,
            key.key_prefix,
            key.updated_at
        ],
    )?;
    get(conn, &key.id)
}

pub fn activate(conn: &Connection, site_id: &str, api_key_id: &str) -> AppResult<SiteApiKeyRow> {
    let key = get_for_site(conn, site_id, api_key_id)?;
    if key.is_active {
        return Ok(key);
    }
    let now = Utc::now().timestamp_millis();
    conn.execute(
        "UPDATE site_api_keys SET is_active = 0, updated_at = ?2 WHERE site_id = ?1 AND is_active = 1",
        params![site_id, now],
    )?;
    conn.execute(
        "UPDATE site_api_keys SET is_active = 1, updated_at = ?2 WHERE id = ?1",
        params![api_key_id, now],
    )?;
    get(conn, api_key_id)
}

pub fn delete(conn: &Connection, site_id: &str, api_key_id: &str) -> AppResult<()> {
    let key = get_for_site(conn, site_id, api_key_id)?;
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM site_api_keys WHERE site_id = ?1",
        params![site_id],
        |r| r.get(0),
    )?;
    if count <= 1 {
        return Err(AppError::new(
            "validation_failed",
            "the last API key cannot be deleted",
        ));
    }
    if key.is_active {
        return Err(AppError::new(
            "validation_failed",
            "switch to another API key before deleting the current one",
        ));
    }
    let live: i64 = conn.query_row(
        "SELECT COUNT(*) FROM target_bindings WHERE site_api_key_id = ?1 AND orphan = 0",
        params![api_key_id],
        |r| r.get(0),
    )?;
    if live > 0 {
        return Err(AppError::new(
            "validation_failed",
            "this API key is still applied to a target; sync or restore the target first",
        ));
    }
    conn.execute(
        "DELETE FROM site_api_keys WHERE id = ?1",
        params![api_key_id],
    )?;
    Ok(())
}

pub fn set_selected_model(
    conn: &Connection,
    api_key_id: &str,
    model_id: Option<&str>,
) -> AppResult<()> {
    let now = Utc::now().timestamp_millis();
    conn.execute(
        "UPDATE site_api_keys SET selected_model_id = ?2, updated_at = ?3 WHERE id = ?1",
        params![api_key_id, model_id, now],
    )?;
    Ok(())
}

pub fn update_fetch_meta(
    conn: &Connection,
    api_key_id: &str,
    latency_ms: i64,
    error: Option<&str>,
) -> AppResult<()> {
    let now = Utc::now().timestamp_millis();
    conn.execute(
        "UPDATE site_api_keys SET last_model_fetch_at=?2, last_model_fetch_latency_ms=?3, last_model_fetch_error=?4, updated_at=?2 WHERE id=?1",
        params![api_key_id, now, latency_ms, error],
    )?;
    Ok(())
}

pub fn quota_revision(key: &SiteApiKeyRow) -> String {
    key_fingerprint(&key.api_key_encrypted)
}

fn optional_text(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|s| !s.is_empty())
}

pub fn sync_for_site(
    conn: &Connection,
    crypto: &Crypto,
    site_id: &str,
    items: &[UpsertSiteApiKeyInput],
) -> AppResult<()> {
    if items.is_empty() {
        return Err(AppError::new("validation_failed", "API key is required"));
    }
    let existing = list_for_site(conn, site_id)?;
    let existing_ids: HashSet<&str> = existing.iter().map(|key| key.id.as_str()).collect();
    let mut assigned: Vec<Option<String>> = Vec::with_capacity(items.len());
    let mut keep = HashSet::new();
    for item in items {
        if optional_text(Some(item.api_key.as_str())).is_none() {
            return Err(AppError::new("validation_failed", "API key is required"));
        }
        match optional_text(item.id.as_deref()) {
            Some(id) if existing_ids.contains(id) => {
                keep.insert(id.to_string());
                assigned.push(Some(id.to_string()));
            }
            Some(_) => {
                return Err(AppError::new(
                    "validation_failed",
                    "api key does not belong to this site",
                ));
            }
            None => assigned.push(None),
        }
    }

    let first_kept = assigned.iter().flatten().next().cloned();
    if let Some(id) = first_kept {
        let active = get_active(conn, site_id)?;
        if active.id != id {
            activate(conn, site_id, &id)?;
        }
    } else {
        let first = &items[0];
        let label = optional_text(first.label.as_deref());
        let row = add(conn, crypto, site_id, label, &first.api_key)?;
        activate(conn, site_id, &row.id)?;
        keep.insert(row.id.clone());
        assigned[0] = Some(row.id);
    }

    for key in &existing {
        if !keep.contains(&key.id) {
            delete(conn, site_id, &key.id)?;
        }
    }

    for (index, item) in items.iter().enumerate() {
        if let Some(id) = assigned[index].clone() {
            let current = get_for_site(conn, site_id, &id)?;
            let next_label = optional_text(item.label.as_deref()).filter(|label| *label != current.label.as_str());
            let current_secret = decrypt(crypto, &current)?;
            let next_secret = optional_text(Some(item.api_key.as_str())).filter(|secret| *secret != current_secret.as_str());
            if next_label.is_some() || next_secret.is_some() {
                update(conn, crypto, site_id, &id, next_label, next_secret)?;
            }
            continue;
        }
        let label = optional_text(item.label.as_deref());
        let row = add(conn, crypto, site_id, label, &item.api_key)?;
        assigned[index] = Some(row.id);
    }

    let first_id = assigned[0]
        .as_deref()
        .ok_or_else(|| AppError::new("internal", "synced api key is missing"))?;
    let active = get_active(conn, site_id)?;
    if active.id != first_id {
        activate(conn, site_id, first_id)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::CreateSiteInput;

    fn setup() -> (Connection, Crypto, String) {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::apply_schema(&conn).unwrap();
        let crypto = Crypto::from_key([3u8; 32]);
        let site = crate::repo::site::create_site(
            &conn,
            &crypto,
            CreateSiteInput {
                newapi_access_token: None,
                newapi_user_id: None,
                name: "Relay".into(),
                base_url: "https://a.example.com".into(),
                base_urls: None,
                api_key: "sk-one".into(),
                api_key_label: None,
                extra_api_keys: Vec::new(),
                protocol: None,
                claude_auth_key_style: None,
                notes: None,
                capabilities: None,
                proxy_headers: None,
            },
        )
        .unwrap();
        (conn, crypto, site.id)
    }

    #[test]
    fn site_always_has_exactly_one_active_key() {
        let (conn, crypto, site_id) = setup();
        let second = add(&conn, &crypto, &site_id, None, "sk-two").unwrap();
        assert!(!second.is_active);
        assert_eq!(second.label, "K 2");
        activate(&conn, &site_id, &second.id).unwrap();
        let keys = list_for_site(&conn, &site_id).unwrap();
        assert_eq!(keys.iter().filter(|k| k.is_active).count(), 1);
        assert_eq!(get_active(&conn, &site_id).unwrap().id, second.id);
    }

    #[test]
    fn rejects_duplicate_label_and_secret() {
        let (conn, crypto, site_id) = setup();
        let err = add(&conn, &crypto, &site_id, Some("K 1"), "sk-two").unwrap_err();
        assert!(err.to_string().contains("already exists"));
        let err = add(&conn, &crypto, &site_id, Some("K 2"), "sk-one").unwrap_err();
        assert!(err.to_string().contains("already exists"));
    }

    #[test]
    fn rejects_cross_site_and_last_or_active_delete() {
        let (conn, crypto, site_id) = setup();
        let other = crate::repo::site::create_site(
            &conn,
            &crypto,
            CreateSiteInput {
                newapi_access_token: None,
                newapi_user_id: None,
                name: "Other".into(),
                base_url: "https://b.example.com".into(),
                base_urls: None,
                api_key: format!("test-{}-other", "key"),
                api_key_label: None,
                extra_api_keys: Vec::new(),
                protocol: None,
                claude_auth_key_style: None,
                notes: None,
                capabilities: None,
                proxy_headers: None,
            },
        )
        .unwrap();
        let active = get_active(&conn, &site_id).unwrap();
        assert!(get_for_site(&conn, &other.id, &active.id).is_err());
        assert!(delete(&conn, &site_id, &active.id).is_err());
        let second = add(&conn, &crypto, &site_id, None, "sk-two").unwrap();
        assert!(delete(&conn, &site_id, &active.id).is_err());
        delete(&conn, &site_id, &second.id).unwrap();
        assert_eq!(list_for_site(&conn, &site_id).unwrap().len(), 1);
    }

    #[test]
    fn models_for_two_keys_stay_isolated() {
        let (conn, crypto, site_id) = setup();
        let first = get_active(&conn, &site_id).unwrap();
        let second = add(&conn, &crypto, &site_id, None, "sk-two").unwrap();
        crate::repo::site::replace_models(
            &conn,
            &site_id,
            &first.id,
            &[crate::domain::SiteModelDto {
                id: "m1".into(),
                site_id: site_id.clone(),
                api_key_id: first.id.clone(),
                model_id: "gpt-4.1".into(),
                display_name: "gpt-4.1".into(),
                owned_by: None,
                raw: None,
                is_manual: false,
            }],
        )
        .unwrap();
        crate::repo::site::replace_models(
            &conn,
            &site_id,
            &second.id,
            &[crate::domain::SiteModelDto {
                id: "m2".into(),
                site_id: site_id.clone(),
                api_key_id: second.id.clone(),
                model_id: "gpt-4.1".into(),
                display_name: "k2-gpt".into(),
                owned_by: None,
                raw: None,
                is_manual: false,
            }],
        )
        .unwrap();
        let a = crate::repo::site::list_models_for_key(&conn, &first.id).unwrap();
        let b = crate::repo::site::list_models_for_key(&conn, &second.id).unwrap();
        assert_eq!(a[0].display_name, "gpt-4.1");
        assert_eq!(b[0].display_name, "k2-gpt");
        crate::repo::site::set_selected_model_for_key(&conn, &site_id, &first.id, "gpt-4.1")
            .unwrap();
        activate(&conn, &site_id, &second.id).unwrap();
        assert_eq!(
            crate::repo::site::get_site(&conn, &site_id)
                .unwrap()
                .selected_model_id
                .as_deref(),
            Some("gpt-4.1")
        );
        assert_eq!(
            get(&conn, &first.id).unwrap().selected_model_id.as_deref(),
            Some("gpt-4.1")
        );
    }
}
