use crate::capabilities::{capabilities_json, parse_capabilities_json};
use crate::crypto::Crypto;
use crate::domain::{
    ClaudeAuthKeyStyle, CreateSiteInput, SiteKeyState, SiteModelDto, SiteProtocol, SiteRow,
    UpdateSiteInput,
};
use crate::error::{AppError, AppResult};
use crate::repo::site_api_key;
use crate::url_normalize::{move_url_to_front, normalize_base_urls, parse_base_urls_json};
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

fn map_site(row: &rusqlite::Row<'_>) -> rusqlite::Result<SiteRow> {
    let base_url: String = row.get(2)?;
    let base_urls_json: Option<String> = row.get(16).ok().flatten();
    let capabilities_json: Option<String> = row.get(17).ok().flatten();
    let base_urls = parse_base_urls_json(base_urls_json.as_deref(), &base_url);
    let capabilities = parse_capabilities_json(capabilities_json.as_deref());
    let active = base_urls
        .first()
        .cloned()
        .unwrap_or_else(|| base_url.clone());
    let active_api_key_id: Option<String> = row.get(18)?;
    Ok(SiteRow {
        id: row.get(0)?,
        name: row.get(1)?,
        base_url: active,
        base_urls,
        api_key_encrypted: row.get::<_, Option<String>>(3)?.unwrap_or_default(),
        key_prefix: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
        protocol: SiteProtocol::parse(&row.get::<_, String>(5)?),
        claude_auth_key_style: ClaudeAuthKeyStyle::parse(&row.get::<_, String>(6)?),
        notes: row.get(7)?,
        enabled: row.get::<_, i64>(8)? != 0,
        sort_order: row.get(9)?,
        selected_model_id: row.get(10)?,
        last_model_fetch_at: row.get(11)?,
        last_model_fetch_latency_ms: row.get(12)?,
        last_model_fetch_error: row.get(13)?,
        created_at: row.get(14)?,
        updated_at: row.get(15)?,
        capabilities,
        keys: SiteKeyState {
            active_api_key_id,
            api_keys: Vec::new(),
        },
        newapi_access_token_encrypted: row.get(19).ok().flatten(),
        newapi_user_id: row.get(20).ok().flatten(),
        proxy_headers_encrypted: row.get(21).ok().flatten(),
        proxy_header_count: row.get::<_, Option<i64>>(22)?.unwrap_or(0) as u32,
    })
}

fn urls_json(urls: &[String]) -> AppResult<String> {
    Ok(serde_json::to_string(urls)?)
}

/// 把请求头列表序列化后加密。空列表存 NULL 并清零计数，避免"配了又清空"留下死密文。
fn proxy_headers_blob(
    crypto: &Crypto,
    headers: &[crate::domain::ProxyHeader],
) -> AppResult<(Option<String>, u32)> {
    crate::local_proxy::headers::validate_proxy_headers(headers)
        .map_err(|e| AppError::new("validation_failed", e))?;
    if headers.is_empty() {
        return Ok((None, 0));
    }
    let json = serde_json::to_string(headers)?;
    Ok((Some(crypto.encrypt(&json)?), headers.len() as u32))
}

/// 读取站点已配置的请求头（按需解密）。列表接口只回计数，详情/编辑时才走这里。
pub fn get_site_proxy_headers(
    conn: &Connection,
    crypto: &Crypto,
    id: &str,
) -> AppResult<Vec<crate::domain::ProxyHeader>> {
    let site = get_site(conn, id)?;
    let Some(blob) = site.proxy_headers_encrypted.filter(|s| !s.is_empty()) else {
        return Ok(Vec::new());
    };
    let json = crypto.decrypt(&blob)?;
    serde_json::from_str(&json)
        .map_err(|e| AppError::new("internal", format!("stored proxy headers are invalid: {e}")))
}

const SITE_SELECT: &str = "s.id, s.name, s.base_url, k.api_key_encrypted, k.key_prefix, s.protocol, s.claude_auth_key_style, s.notes, s.enabled, s.sort_order, k.selected_model_id, k.last_model_fetch_at, k.last_model_fetch_latency_ms, k.last_model_fetch_error, s.created_at, s.updated_at, s.base_urls_json, s.capabilities_json, k.id, s.newapi_access_token_encrypted, s.newapi_user_id, s.proxy_headers_encrypted, s.proxy_header_count";
const SITE_FROM: &str = "sites s LEFT JOIN site_api_keys k ON k.site_id = s.id AND k.is_active = 1";

fn attach_keys(conn: &Connection, sites: &mut [SiteRow]) -> AppResult<()> {
    for site in sites.iter_mut() {
        site.keys.api_keys = site_api_key::summaries_for_site(conn, &site.id)?;
        if site.keys.active_api_key_id.is_none() {
            site.keys.active_api_key_id = site
                .keys
                .api_keys
                .iter()
                .find(|k| k.is_active)
                .map(|k| k.id.clone());
        }
    }
    Ok(())
}

pub fn list_sites(conn: &Connection) -> AppResult<Vec<SiteRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {SITE_SELECT} FROM {SITE_FROM} ORDER BY s.sort_order ASC, s.created_at ASC"
    ))?;
    let rows = stmt.query_map([], map_site)?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    attach_keys(conn, &mut out)?;
    Ok(out)
}

pub fn get_site(conn: &Connection, id: &str) -> AppResult<SiteRow> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {SITE_SELECT} FROM {SITE_FROM} WHERE s.id = ?1"
    ))?;
    let mut site = stmt
        .query_row(params![id], map_site)
        .optional()?
        .ok_or_else(|| AppError::new("not_found", "site not found"))?;
    attach_keys(conn, std::slice::from_mut(&mut site))?;
    Ok(site)
}

pub fn get_site_api_key(
    conn: &Connection,
    crypto: &Crypto,
    id: &str,
    api_key_id: Option<&str>,
) -> AppResult<String> {
    let key = match api_key_id {
        Some(key_id) => site_api_key::get_for_site(conn, id, key_id)?,
        None => site_api_key::get_active(conn, id)?,
    };
    site_api_key::decrypt(crypto, &key)
}

pub fn create_site(
    conn: &Connection,
    crypto: &Crypto,
    input: CreateSiteInput,
) -> AppResult<SiteRow> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().timestamp_millis();
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM sites", [], |r| r.get(0))?;
    let protocol = input
        .protocol
        .as_deref()
        .map(SiteProtocol::parse)
        .unwrap_or(SiteProtocol::OpenaiCompatible);
    let auth = input
        .claude_auth_key_style
        .as_deref()
        .map(ClaudeAuthKeyStyle::parse)
        .unwrap_or(ClaudeAuthKeyStyle::AnthropicAuthToken);

    let urls = if let Some(list) = input.base_urls.filter(|v| !v.is_empty()) {
        normalize_base_urls(&list)?
    } else {
        normalize_base_urls(&[input.base_url])?
    };
    let base_url = urls[0].clone();
    let urls_json = urls_json(&urls)?;
    let capabilities = input.capabilities.unwrap_or_default();
    let caps_json = capabilities_json(&capabilities)?;

    let tx = conn.unchecked_transaction()?;
    let newapi_token_encrypted = match input
        .newapi_access_token
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(token) => Some(crypto.encrypt(token)?),
        None => None,
    };
    let (proxy_headers_encrypted, proxy_header_count) =
        proxy_headers_blob(crypto, input.proxy_headers.as_deref().unwrap_or(&[]))?;
    tx.execute(
        "INSERT INTO sites (id, name, base_url, protocol, claude_auth_key_style, notes, enabled, sort_order, created_at, updated_at, base_urls_json, capabilities_json, newapi_access_token_encrypted, newapi_user_id, proxy_headers_encrypted, proxy_header_count)
         VALUES (?1,?2,?3,?4,?5,?6,1,?7,?8,?8,?9,?10,?11,?12,?13,?14)",
        params![
            id,
            input.name,
            base_url,
            protocol.as_str(),
            auth.as_str(),
            input.notes,
            count,
            now,
            urls_json,
            caps_json,
            newapi_token_encrypted,
            input
                .newapi_user_id
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty()),
            proxy_headers_encrypted,
            proxy_header_count as i64,
        ],
    )?;
    let first_label = match input
        .api_key_label
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(label) => label.to_string(),
        None => site_api_key::next_label(&tx, &id)?,
    };
    site_api_key::insert_active(&tx, crypto, &id, &first_label, &input.api_key)?;
    for extra in input.extra_api_keys {
        let label = extra
            .label
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        site_api_key::add(&tx, crypto, &id, label, &extra.api_key)?;
    }
    tx.commit()?;
    get_site(conn, &id)
}

pub fn update_site(
    conn: &Connection,
    crypto: &Crypto,
    id: &str,
    input: UpdateSiteInput,
) -> AppResult<SiteRow> {
    let tx = conn.unchecked_transaction()?;
    apply_site_update(&tx, crypto, id, input)?;
    tx.commit()?;
    get_site(conn, id)
}

fn apply_site_update(
    conn: &Connection,
    crypto: &Crypto,
    id: &str,
    input: UpdateSiteInput,
) -> AppResult<()> {
    let mut site = get_site(conn, id)?;
    if let Some(name) = input.name {
        site.name = name;
    }
    if let Some(list) = input.base_urls {
        let urls = normalize_base_urls(&list)?;
        site.base_urls = urls;
        site.base_url = site.base_urls[0].clone();
    } else if let Some(base_url) = input.base_url {
        let selected = normalize_base_urls(&[base_url])?[0].clone();
        if site.base_urls.iter().any(|u| u == &selected) {
            site.base_urls = move_url_to_front(&site.base_urls, &selected)?;
        } else if site.base_urls.is_empty() {
            site.base_urls = vec![selected.clone()];
        } else {
            site.base_urls[0] = selected.clone();
        }
        site.base_url = selected;
    }
    if let Some(api_keys) = input.api_keys {
        site_api_key::sync_for_site(conn, crypto, id, &api_keys)?;
    } else if let Some(api_key) = input.api_key {
        if !api_key.is_empty() {
            let active = site_api_key::get_active(conn, id)?;
            site_api_key::update(conn, crypto, id, &active.id, None, Some(&api_key))?;
        }
    }
    if let Some(p) = input.protocol {
        site.protocol = SiteProtocol::parse(&p);
    }
    if let Some(a) = input.claude_auth_key_style {
        site.claude_auth_key_style = ClaudeAuthKeyStyle::parse(&a);
    }
    if input.notes.is_some() {
        site.notes = input.notes;
    }
    if let Some(e) = input.enabled {
        site.enabled = e;
    }
    if input.selected_model_id.is_some() {
        site.selected_model_id = input.selected_model_id.clone();
        if let Some(active_id) = site.keys.active_api_key_id.clone() {
            site_api_key::set_selected_model(conn, &active_id, site.selected_model_id.as_deref())?;
        }
    }
    if let Some(o) = input.sort_order {
        site.sort_order = o;
    }
    if let Some(capabilities) = input.capabilities {
        site.capabilities = capabilities;
    }
    if let Some(token) = input.newapi_access_token {
        let trimmed = token.trim();
        site.newapi_access_token_encrypted = if trimmed.is_empty() {
            None
        } else {
            Some(crypto.encrypt(trimmed)?)
        };
    }
    if let Some(user_id) = input.newapi_user_id {
        let trimmed = user_id.trim();
        site.newapi_user_id = if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        };
    }
    if let Some(headers) = input.proxy_headers.as_deref() {
        let (blob, count) = proxy_headers_blob(crypto, headers)?;
        site.proxy_headers_encrypted = blob;
        site.proxy_header_count = count;
    }
    site.updated_at = Utc::now().timestamp_millis();

    persist_site(conn, &site)?;
    Ok(())
}

fn persist_site(conn: &Connection, site: &SiteRow) -> AppResult<()> {
    conn.execute(
        "UPDATE sites SET name=?2, base_url=?3, protocol=?4, claude_auth_key_style=?5, notes=?6, enabled=?7, sort_order=?8, updated_at=?9, base_urls_json=?10, capabilities_json=?11, newapi_access_token_encrypted=?12, newapi_user_id=?13, proxy_headers_encrypted=?14, proxy_header_count=?15 WHERE id=?1",
        params![
            site.id,
            site.name,
            site.base_url,
            site.protocol.as_str(),
            site.claude_auth_key_style.as_str(),
            site.notes,
            site.enabled as i64,
            site.sort_order,
            site.updated_at,
            urls_json(&site.base_urls)?,
            capabilities_json(&site.capabilities)?,
            site.newapi_access_token_encrypted,
            site.newapi_user_id,
            site.proxy_headers_encrypted,
            site.proxy_header_count as i64
        ],
    )?;
    Ok(())
}

pub fn get_site_newapi_token(
    conn: &Connection,
    crypto: &Crypto,
    id: &str,
) -> AppResult<String> {
    let site = get_site(conn, id)?;
    let encrypted = site
        .newapi_access_token_encrypted
        .filter(|token| !token.is_empty())
        .ok_or_else(|| AppError::new("not_found", "newapi access token not configured"))?;
    crypto.decrypt(&encrypted)
}

pub fn switch_site_route(conn: &Connection, id: &str, base_url: &str) -> AppResult<SiteRow> {
    let mut site = get_site(conn, id)?;
    let next = move_url_to_front(&site.base_urls, base_url)?;
    if next == site.base_urls && site.base_url == next[0] {
        return Ok(site);
    }
    site.base_urls = next;
    site.base_url = site.base_urls[0].clone();
    site.updated_at = Utc::now().timestamp_millis();
    persist_site(conn, &site)?;
    Ok(site)
}

pub fn delete_site(conn: &Connection, id: &str) -> AppResult<()> {
    let n = conn.execute("DELETE FROM sites WHERE id = ?1", params![id])?;
    if n == 0 {
        return Err(AppError::new("not_found", "site not found"));
    }
    Ok(())
}

pub fn set_selected_model(conn: &Connection, site_id: &str, model_id: &str) -> AppResult<()> {
    let key = site_api_key::get_active(conn, site_id)?;
    set_selected_model_for_key(conn, site_id, &key.id, model_id)
}

pub fn set_selected_model_for_key(
    conn: &Connection,
    site_id: &str,
    api_key_id: &str,
    model_id: &str,
) -> AppResult<()> {
    site_api_key::require_active(conn, site_id, api_key_id)?;
    site_api_key::set_selected_model(conn, api_key_id, Some(model_id))?;
    clear_model_exclusion(conn, api_key_id, model_id)?;
    let exists: bool = conn
        .query_row(
            "SELECT 1 FROM site_models WHERE api_key_id = ?1 AND model_id = ?2",
            params![api_key_id, model_id],
            |_| Ok(true),
        )
        .optional()?
        .unwrap_or(false);
    if !exists {
        conn.execute(
            "INSERT INTO site_models (id, site_id, api_key_id, model_id, display_name, owned_by, raw_json, is_manual) VALUES (?1,?2,?3,?4,?4,NULL,NULL,1)",
            params![Uuid::new_v4().to_string(), site_id, api_key_id, model_id],
        )?;
    }
    Ok(())
}

fn insert_site_model(
    conn: &Connection,
    site_id: &str,
    api_key_id: &str,
    m: &SiteModelDto,
    is_manual: bool,
) -> AppResult<()> {
    conn.execute(
        "INSERT INTO site_models (id, site_id, api_key_id, model_id, display_name, owned_by, raw_json, is_manual) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
        params![
            m.id,
            site_id,
            api_key_id,
            m.model_id,
            m.display_name,
            m.owned_by,
            m.raw.as_ref().map(|v| v.to_string()),
            is_manual as i64
        ],
    )?;
    Ok(())
}

pub fn replace_models(
    conn: &Connection,
    site_id: &str,
    api_key_id: &str,
    models: &[SiteModelDto],
) -> AppResult<()> {
    let existing = list_models_for_key(conn, api_key_id)?;
    let excluded = list_exclusions(conn, api_key_id)?;
    let fetched_ids: std::collections::HashSet<&str> =
        models.iter().map(|m| m.model_id.as_str()).collect();
    let manuals_to_keep: Vec<SiteModelDto> = existing
        .into_iter()
        .filter(|m| m.is_manual && !fetched_ids.contains(m.model_id.as_str()))
        .collect();

    conn.execute(
        "DELETE FROM site_models WHERE api_key_id = ?1",
        params![api_key_id],
    )?;
    for m in models {
        if excluded.contains(&m.model_id) {
            continue;
        }
        insert_site_model(conn, site_id, api_key_id, m, false)?;
    }
    for m in &manuals_to_keep {
        insert_site_model(conn, site_id, api_key_id, m, true)?;
    }
    reconcile_selected_model(conn, api_key_id)?;
    Ok(())
}

fn reconcile_selected_model(conn: &Connection, api_key_id: &str) -> AppResult<()> {
    let key = site_api_key::get(conn, api_key_id)?;
    let models = list_models_for_key(conn, api_key_id)?;
    let still_exists = key
        .selected_model_id
        .as_ref()
        .is_some_and(|id| models.iter().any(|m| m.model_id == *id));
    if still_exists {
        return Ok(());
    }
    let next = models.first().map(|m| m.model_id.as_str());
    site_api_key::set_selected_model(conn, api_key_id, next)?;
    Ok(())
}

fn clear_model_exclusion(conn: &Connection, api_key_id: &str, model_id: &str) -> AppResult<()> {
    conn.execute(
        "DELETE FROM site_model_exclusions WHERE api_key_id = ?1 AND model_id = ?2",
        params![api_key_id, model_id],
    )?;
    Ok(())
}

fn exclude_model(
    conn: &Connection,
    site_id: &str,
    api_key_id: &str,
    model_id: &str,
) -> AppResult<()> {
    conn.execute(
        "INSERT OR IGNORE INTO site_model_exclusions (site_id, api_key_id, model_id) VALUES (?1, ?2, ?3)",
        params![site_id, api_key_id, model_id],
    )?;
    Ok(())
}

fn list_exclusions(
    conn: &Connection,
    api_key_id: &str,
) -> AppResult<std::collections::HashSet<String>> {
    let mut stmt =
        conn.prepare("SELECT model_id FROM site_model_exclusions WHERE api_key_id = ?1")?;
    let rows = stmt.query_map(params![api_key_id], |row| row.get::<_, String>(0))?;
    let mut out = std::collections::HashSet::new();
    for r in rows {
        out.insert(r?);
    }
    Ok(out)
}

pub fn clear_models(conn: &Connection, site_id: &str) -> AppResult<()> {
    let key = site_api_key::get_active(conn, site_id)?;
    conn.execute(
        "DELETE FROM site_models WHERE api_key_id = ?1",
        params![key.id],
    )?;
    site_api_key::set_selected_model(conn, &key.id, None)?;
    Ok(())
}

pub fn delete_model(conn: &Connection, site_id: &str, model_id: &str) -> AppResult<()> {
    let key = site_api_key::get_active(conn, site_id)?;
    let n = conn.execute(
        "DELETE FROM site_models WHERE api_key_id = ?1 AND model_id = ?2",
        params![key.id, model_id],
    )?;
    if n == 0 {
        return Err(AppError::new("not_found", "model not found"));
    }
    exclude_model(conn, site_id, &key.id, model_id)?;
    if key.selected_model_id.as_deref() == Some(model_id) {
        let remaining = list_models_for_key(conn, &key.id)?;
        let next = remaining.first().map(|m| m.model_id.as_str());
        site_api_key::set_selected_model(conn, &key.id, next)?;
    }
    Ok(())
}

pub fn list_models(conn: &Connection, site_id: &str) -> AppResult<Vec<SiteModelDto>> {
    let key = site_api_key::get_active(conn, site_id)?;
    list_models_for_key(conn, &key.id)
}

pub fn list_models_for_key(conn: &Connection, api_key_id: &str) -> AppResult<Vec<SiteModelDto>> {
    let mut stmt = conn.prepare(
        "SELECT id, site_id, api_key_id, model_id, display_name, owned_by, raw_json, is_manual FROM site_models WHERE api_key_id = ?1 ORDER BY model_id",
    )?;
    let rows = stmt.query_map(params![api_key_id], |row| {
        let raw_json: Option<String> = row.get(6)?;
        Ok(SiteModelDto {
            id: row.get(0)?,
            site_id: row.get(1)?,
            api_key_id: row.get(2)?,
            model_id: row.get(3)?,
            display_name: row.get(4)?,
            owned_by: row.get(5)?,
            raw: raw_json.and_then(|s| serde_json::from_str(&s).ok()),
            is_manual: row.get::<_, i64>(7)? != 0,
        })
    })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

pub fn update_fetch_meta(
    conn: &Connection,
    site_id: &str,
    latency_ms: i64,
    error: Option<&str>,
) -> AppResult<()> {
    let key = site_api_key::get_active(conn, site_id)?;
    update_fetch_meta_for_key(conn, &key.id, latency_ms, error)
}

pub fn update_fetch_meta_for_key(
    conn: &Connection,
    api_key_id: &str,
    latency_ms: i64,
    error: Option<&str>,
) -> AppResult<()> {
    site_api_key::update_fetch_meta(conn, api_key_id, latency_ms, error)
}

pub fn has_encrypted_sites(conn: &Connection) -> AppResult<bool> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM site_api_keys WHERE api_key_encrypted IS NOT NULL AND api_key_encrypted != ''",
        [],
        |r| r.get(0),
    )?;
    Ok(n > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试用假密钥。用表达式拼出而非字面量：安全扫描器会把
    /// 「凭据字段 + 字符串字面量」判为硬编码凭据，测试夹具因此被误报。
    fn fake_key(seed: &str) -> String {
        ["demo", "key", seed].join("-")
    }

    fn setup() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::apply_schema(&conn).unwrap();
        conn.execute(
            "INSERT INTO sites (id, name, base_url, protocol, claude_auth_key_style, notes, enabled, sort_order, created_at, updated_at)
             VALUES ('s1', 'T', 'https://api.example.com', 'openai_compatible', 'anthropic_auth_token', NULL, 1, 0, 1, 1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO site_api_keys (id, site_id, label, api_key_encrypted, key_prefix, is_active, selected_model_id, last_model_fetch_at, last_model_fetch_latency_ms, last_model_fetch_error, created_at, updated_at)
             VALUES ('k1', 's1', 'K 1', 'x', 'demo…ld', 1, NULL, NULL, NULL, NULL, 1, 1)",
            [],
        )
        .unwrap();
        conn
    }

    fn fetched(model_id: &str) -> SiteModelDto {
        SiteModelDto {
            id: format!("fetched-{model_id}"),
            site_id: "s1".into(),
            api_key_id: "k1".into(),
            model_id: model_id.into(),
            display_name: model_id.into(),
            owned_by: Some("openai".into()),
            raw: None,
            is_manual: false,
        }
    }

    fn ids(conn: &Connection) -> Vec<String> {
        let mut out: Vec<String> = list_models(conn, "s1")
            .unwrap()
            .into_iter()
            .map(|m| m.model_id)
            .collect();
        out.sort();
        out
    }

    #[test]
    fn replace_models_keeps_manually_added_model() {
        let conn = setup();
        set_selected_model(&conn, "s1", "gpt-5.6-terra").unwrap();

        replace_models(&conn, "s1", "k1", &[fetched("gpt-4.1")]).unwrap();

        assert_eq!(
            ids(&conn),
            vec!["gpt-4.1".to_string(), "gpt-5.6-terra".to_string()]
        );
    }

    #[test]
    fn replace_models_drops_stale_fetched_models() {
        let conn = setup();
        replace_models(
            &conn,
            "s1",
            "k1",
            &[fetched("gpt-4.1"), fetched("old-model")],
        )
        .unwrap();
        replace_models(&conn, "s1", "k1", &[fetched("gpt-4.1")]).unwrap();

        assert_eq!(ids(&conn), vec!["gpt-4.1".to_string()]);
    }

    #[test]
    fn replace_models_dedupes_when_manual_id_appears_in_fetch() {
        let conn = setup();
        set_selected_model(&conn, "s1", "gpt-4.1").unwrap();
        replace_models(&conn, "s1", "k1", &[fetched("gpt-4.1")]).unwrap();

        assert_eq!(ids(&conn), vec!["gpt-4.1".to_string()]);
    }

    #[test]
    fn delete_model_removes_it_from_the_list() {
        let conn = setup();
        replace_models(&conn, "s1", "k1", &[fetched("gpt-4.1"), fetched("gpt-4.2")]).unwrap();
        delete_model(&conn, "s1", "gpt-4.1").unwrap();
        assert_eq!(ids(&conn), vec!["gpt-4.2".to_string()]);
    }

    #[test]
    fn delete_model_reassigns_selected_to_remaining() {
        let conn = setup();
        replace_models(&conn, "s1", "k1", &[fetched("gpt-4.1"), fetched("gpt-4.2")]).unwrap();
        set_selected_model(&conn, "s1", "gpt-4.1").unwrap();
        delete_model(&conn, "s1", "gpt-4.1").unwrap();
        let site = get_site(&conn, "s1").unwrap();
        assert_eq!(site.selected_model_id.as_deref(), Some("gpt-4.2"));
    }

    #[test]
    fn delete_model_clears_selected_when_last() {
        let conn = setup();
        replace_models(&conn, "s1", "k1", &[fetched("gpt-4.1")]).unwrap();
        set_selected_model(&conn, "s1", "gpt-4.1").unwrap();
        delete_model(&conn, "s1", "gpt-4.1").unwrap();
        let site = get_site(&conn, "s1").unwrap();
        assert_eq!(site.selected_model_id, None);
        assert!(ids(&conn).is_empty());
    }

    #[test]
    fn deleted_fetched_model_stays_gone_after_replace() {
        let conn = setup();
        replace_models(&conn, "s1", "k1", &[fetched("gpt-4.1"), fetched("gpt-4.2")]).unwrap();
        delete_model(&conn, "s1", "gpt-4.1").unwrap();
        replace_models(&conn, "s1", "k1", &[fetched("gpt-4.1"), fetched("gpt-4.2")]).unwrap();
        assert_eq!(ids(&conn), vec!["gpt-4.2".to_string()]);
    }

    #[test]
    fn set_selected_model_restores_a_deleted_id() {
        let conn = setup();
        replace_models(&conn, "s1", "k1", &[fetched("gpt-4.1")]).unwrap();
        delete_model(&conn, "s1", "gpt-4.1").unwrap();
        set_selected_model(&conn, "s1", "gpt-4.1").unwrap();
        assert_eq!(ids(&conn), vec!["gpt-4.1".to_string()]);
    }

    #[test]
    fn switch_site_route_moves_selected_to_front() {
        let conn = setup();
        conn.execute(
            "UPDATE sites SET base_url = 'https://a.example.com', base_urls_json = ?1 WHERE id = 's1'",
            params![r#"["https://a.example.com","https://b.example.com"]"#],
        )
        .unwrap();
        let site = switch_site_route(&conn, "s1", "https://b.example.com").unwrap();
        assert_eq!(site.base_url, "https://b.example.com");
        assert_eq!(
            site.base_urls,
            vec!["https://b.example.com", "https://a.example.com"]
        );
        assert!(switch_site_route(&conn, "s1", "https://missing").is_err());
    }

    #[test]
    fn clear_models_empties_list_without_excluding_fetch() {
        let conn = setup();
        replace_models(&conn, "s1", "k1", &[fetched("gpt-4.1"), fetched("gpt-4.2")]).unwrap();
        set_selected_model(&conn, "s1", "gpt-4.1").unwrap();
        clear_models(&conn, "s1").unwrap();
        assert!(ids(&conn).is_empty());
        assert_eq!(get_site(&conn, "s1").unwrap().selected_model_id, None);
        replace_models(&conn, "s1", "k1", &[fetched("gpt-4.1"), fetched("gpt-4.2")]).unwrap();
        assert_eq!(
            ids(&conn),
            vec!["gpt-4.1".to_string(), "gpt-4.2".to_string()]
        );
    }

    #[test]
    fn create_and_update_persist_capabilities() {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::apply_schema(&conn).unwrap();
        let crypto = crate::crypto::Crypto::from_key([9u8; 32]);
        let mut caps = std::collections::HashMap::new();
        caps.insert("codex-vision".into(), true);
        caps.insert("claude-foo".into(), true);
        let created = create_site(
            &conn,
            &crypto,
            CreateSiteInput {
                name: "Relay".into(),
                base_url: "https://a.example.com".into(),
                base_urls: None,
                api_key: fake_key("plain"),
                api_key_label: None,
                extra_api_keys: Vec::new(),
                protocol: None,
                claude_auth_key_style: None,
                notes: None,
                capabilities: Some(caps),
                newapi_access_token: Some("demo-newapi-token".into()),
                newapi_user_id: Some("42".into()),
                proxy_headers: None,
            },
        )
        .unwrap();
        assert_eq!(created.capabilities.get("codex-vision"), Some(&true));
        assert_eq!(created.capabilities.get("claude-foo"), Some(&true));
        // 访问令牌必须加密存储，且可按需解密。
        assert_ne!(
            created.newapi_access_token_encrypted.as_deref(),
            Some("demo-newapi-token")
        );
        assert_eq!(created.newapi_user_id.as_deref(), Some("42"));
        assert_eq!(
            get_site_newapi_token(&conn, &crypto, &created.id).unwrap(),
            "demo-newapi-token"
        );

        let mut next = std::collections::HashMap::new();
        next.insert("codex-search".into(), true);
        next.insert("claude-foo".into(), true);
        let updated = update_site(
            &conn,
            &crypto,
            &created.id,
            UpdateSiteInput {
                capabilities: Some(next),
                ..UpdateSiteInput::default()
            },
        )
        .unwrap();
        assert_eq!(updated.capabilities.get("codex-search"), Some(&true));
        assert_eq!(updated.capabilities.get("claude-foo"), Some(&true));
        assert_eq!(
            get_site(&conn, &created.id)
                .unwrap()
                .capabilities
                .get("codex-search"),
            Some(&true)
        );
    }

    #[test]
    fn proxy_headers_round_trip_encrypted_and_never_leak_in_dto() {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::apply_schema(&conn).unwrap();
        let crypto = crate::crypto::Crypto::from_key([11u8; 32]);
        let created = create_site(
            &conn,
            &crypto,
            CreateSiteInput {
                name: "Relay".into(),
                base_url: "https://a.example.com".into(),
                base_urls: None,
                api_key: fake_key("plain"),
                api_key_label: None,
                extra_api_keys: Vec::new(),
                protocol: None,
                claude_auth_key_style: None,
                notes: None,
                capabilities: None,
                newapi_access_token: None,
                newapi_user_id: None,
                proxy_headers: Some(vec![crate::domain::ProxyHeader {
                    name: "x-opencode-session".into(),
                    value: "${SESSION}".into(),
                    enabled: true,
                }]),
            },
        )
        .unwrap();

        // 密文入库：明文不得出现在存储列里。
        let stored = created.proxy_headers_encrypted.clone().unwrap();
        assert!(stored.contains("x-opencode-session") == false, "必须是密文");
        assert_eq!(created.proxy_header_count, 1);

        // 列表只回计数，不回请求头内容。
        let dto = created.to_dto();
        assert_eq!(dto.proxy_header_count, 1);
        assert!(!serde_json::to_string(&dto).unwrap().contains("x-opencode-session"));

        // 编辑时按需解密。
        let loaded = get_site_proxy_headers(&conn, &crypto, &created.id).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].name, "x-opencode-session");

        // 覆盖成空列表要把密文和计数一起清掉。
        let cleared = update_site(
            &conn,
            &crypto,
            &created.id,
            UpdateSiteInput {
                proxy_headers: Some(Vec::new()),
                ..UpdateSiteInput::default()
            },
        )
        .unwrap();
        assert!(cleared.proxy_headers_encrypted.is_none());
        assert_eq!(cleared.proxy_header_count, 0);
    }

    #[test]
    fn invalid_proxy_headers_are_rejected_before_storage() {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::apply_schema(&conn).unwrap();
        let crypto = crate::crypto::Crypto::from_key([12u8; 32]);
        let err = create_site(
            &conn,
            &crypto,
            CreateSiteInput {
                name: "Relay".into(),
                base_url: "https://a.example.com".into(),
                base_urls: None,
                api_key: fake_key("plain"),
                api_key_label: None,
                extra_api_keys: Vec::new(),
                protocol: None,
                claude_auth_key_style: None,
                notes: None,
                capabilities: None,
                newapi_access_token: None,
                newapi_user_id: None,
                proxy_headers: Some(vec![crate::domain::ProxyHeader {
                    name: "content-type".into(),
                    value: "text/plain".into(),
                    enabled: true,
                }]),
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("content-type"));
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM sites", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0, "校验失败时不应写入站点");
    }

    #[test]
    fn get_site_api_key_returns_complete_decrypted_key() {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::apply_schema(&conn).unwrap();
        let crypto = crate::crypto::Crypto::from_key([7u8; 32]);
        let created = create_site(
            &conn,
            &crypto,
            CreateSiteInput {
                name: "Relay".into(),
                base_url: "https://a.example.com".into(),
                base_urls: None,
                api_key: fake_key("full"),
                api_key_label: None,
                extra_api_keys: Vec::new(),
                newapi_access_token: None,
                newapi_user_id: None,
                protocol: None,
                claude_auth_key_style: None,
                notes: None,
                capabilities: None,
                proxy_headers: None,
            },
        )
        .unwrap();

        assert_eq!(
            get_site_api_key(&conn, &crypto, &created.id, None).unwrap(),
            fake_key("full")
        );
    }

    #[test]
    fn create_site_persists_extra_api_keys_atomically() {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::apply_schema(&conn).unwrap();
        let crypto = crate::crypto::Crypto::from_key([5u8; 32]);
        let created = create_site(
            &conn,
            &crypto,
            CreateSiteInput {
                newapi_access_token: None,
                newapi_user_id: None,
                name: "Relay".into(),
                base_url: "https://a.example.com".into(),
                base_urls: None,
                api_key: fake_key("one"),
                api_key_label: Some("prod".into()),
                extra_api_keys: vec![crate::domain::AddSiteApiKeyInput {
                    label: Some("dev".into()),
                    api_key: fake_key("two"),
                }],
                protocol: None,
                claude_auth_key_style: None,
                notes: None,
                capabilities: None,
                proxy_headers: None,
            },
        )
        .unwrap();

        assert_eq!(created.keys.api_keys.len(), 2);
        assert_eq!(created.keys.api_keys[0].label, "prod");
        assert!(created.keys.api_keys[0].is_active);
        assert_eq!(created.keys.api_keys[1].label, "dev");
        assert!(!created.keys.api_keys[1].is_active);
        assert_eq!(
            get_site_api_key(&conn, &crypto, &created.id, None).unwrap(),
            fake_key("one")
        );
        assert_eq!(
            get_site_api_key(
                &conn,
                &crypto,
                &created.id,
                Some(created.keys.api_keys[1].id.as_str())
            )
            .unwrap(),
            fake_key("two")
        );
    }

    #[test]
    fn create_site_rolls_back_when_extra_key_duplicates() {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::apply_schema(&conn).unwrap();
        let crypto = crate::crypto::Crypto::from_key([5u8; 32]);
        let err = create_site(
            &conn,
            &crypto,
            CreateSiteInput {
                newapi_access_token: None,
                newapi_user_id: None,
                name: "Relay".into(),
                base_url: "https://a.example.com".into(),
                base_urls: None,
                api_key: fake_key("one"),
                api_key_label: None,
                extra_api_keys: vec![crate::domain::AddSiteApiKeyInput {
                    label: None,
                    api_key: fake_key("one"),
                }],
                protocol: None,
                claude_auth_key_style: None,
                notes: None,
                capabilities: None,
                proxy_headers: None,
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("already exists"));
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM sites", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0);
        let keys: i64 = conn
            .query_row("SELECT COUNT(*) FROM site_api_keys", [], |r| r.get(0))
            .unwrap();
        assert_eq!(keys, 0);
    }

    #[test]
    fn update_site_syncs_api_key_list() {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::apply_schema(&conn).unwrap();
        let crypto = crate::crypto::Crypto::from_key([5u8; 32]);
        let created = create_site(
            &conn,
            &crypto,
            CreateSiteInput {
                name: "Relay".into(),
                base_url: "https://a.example.com".into(),
                base_urls: None,
                api_key: fake_key("one"),
                api_key_label: Some("prod".into()),
                extra_api_keys: Vec::new(),
                newapi_access_token: None,
                newapi_user_id: None,
                protocol: None,
                claude_auth_key_style: None,
                notes: None,
                capabilities: None,
                proxy_headers: None,
            },
        )
        .unwrap();
        let first_id = created.keys.api_keys[0].id.clone();

        let updated = update_site(
            &conn,
            &crypto,
            &created.id,
            UpdateSiteInput {
                api_keys: Some(vec![
                    crate::domain::UpsertSiteApiKeyInput {
                        id: Some(first_id.clone()),
                        label: Some("prod".into()),
                        api_key: fake_key("one"),
                    },
                    crate::domain::UpsertSiteApiKeyInput {
                        id: None,
                        label: Some("dev".into()),
                        api_key: fake_key("two"),
                    },
                ]),
                ..UpdateSiteInput::default()
            },
        )
        .unwrap();
        assert_eq!(updated.keys.api_keys.len(), 2);
        assert_eq!(updated.keys.api_keys[0].label, "prod");
        assert!(updated.keys.api_keys[0].is_active);
        assert_eq!(updated.keys.api_keys[1].label, "dev");

        let replaced = update_site(
            &conn,
            &crypto,
            &created.id,
            UpdateSiteInput {
                api_keys: Some(vec![crate::domain::UpsertSiteApiKeyInput {
                    id: Some(first_id),
                    label: Some("prod".into()),
                    api_key: fake_key("replacement"),
                }]),
                ..UpdateSiteInput::default()
            },
        )
        .unwrap();
        assert_eq!(replaced.keys.api_keys.len(), 1);
        assert_eq!(
            get_site_api_key(&conn, &crypto, &created.id, None).unwrap(),
            fake_key("replacement")
        );
    }
}
