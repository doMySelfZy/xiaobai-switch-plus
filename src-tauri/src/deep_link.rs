use crate::capabilities::{capabilities_equal, merge_codex_capabilities};
use crate::crypto::Crypto;
use crate::domain::{
    CreateSiteInput, DeepLinkSiteImportInput, DeepLinkSiteImportResult, SiteProtocol, SiteRow,
    UpdateSiteInput,
};
use crate::error::{AppError, AppResult};
use crate::repo::site;
use crate::state::AppState;
use crate::url_normalize::normalize_base_urls;
use rusqlite::Connection;

pub const MAX_NAME: usize = 128;
pub const MAX_NOTES: usize = 2000;
pub const MAX_ROUTES: usize = 20;
pub const MAX_URL_LEN: usize = 2048;

pub fn parse_deep_link_protocol(value: Option<&str>) -> AppResult<SiteProtocol> {
    match value.map(str::trim).filter(|s| !s.is_empty()) {
        None => Ok(SiteProtocol::OpenaiCompatible),
        Some(raw) => match raw.to_ascii_lowercase().as_str() {
            "openai" | "openai_compatible" => Ok(SiteProtocol::OpenaiCompatible),
            "anthropic" => Ok(SiteProtocol::Anthropic),
            other => Err(AppError::new(
                "validation_failed",
                format!("unsupported protocol: {other}"),
            )),
        },
    }
}

fn url_set_eq(a: &[String], b: &[String]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut sa = a.to_vec();
    let mut sb = b.to_vec();
    sa.sort();
    sb.sort();
    sa == sb
}

fn find_matching_site(
    conn: &Connection,
    protocol: &SiteProtocol,
    urls: &[String],
) -> AppResult<Option<SiteRow>> {
    let sites = site::list_sites(conn)?;
    Ok(sites
        .into_iter()
        .find(|row| &row.protocol == protocol && url_set_eq(&row.base_urls, urls)))
}

pub fn import_site_from_deep_link_conn(
    conn: &Connection,
    crypto: &Crypto,
    input: DeepLinkSiteImportInput,
) -> AppResult<DeepLinkSiteImportResult> {
    let name = input.name.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME {
        return Err(AppError::new(
            "validation_failed",
            "site name is required (max 128 chars)",
        ));
    }

    let api_key = input.api_key.trim();
    if api_key.is_empty() {
        return Err(AppError::new("validation_failed", "API key is required"));
    }

    if input
        .base_urls
        .iter()
        .any(|u| u.chars().count() > MAX_URL_LEN)
    {
        return Err(AppError::new(
            "validation_failed",
            "base URL exceeds 2048 characters",
        ));
    }

    let urls = normalize_base_urls(&input.base_urls)?;
    if urls.len() > MAX_ROUTES {
        return Err(AppError::new(
            "validation_failed",
            "at most 20 base URLs are allowed",
        ));
    }

    let protocol = parse_deep_link_protocol(input.protocol.as_deref())?;
    let notes = input
        .notes
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    if notes
        .as_ref()
        .is_some_and(|s| s.chars().count() > MAX_NOTES)
    {
        return Err(AppError::new(
            "validation_failed",
            "notes exceed 2000 characters",
        ));
    }

    if let Some(existing) = find_matching_site(conn, &protocol, &urls)? {
        let keys = crate::repo::site_api_key::list_for_site(conn, &existing.id)?;
        let mut matched_key = None;
        for key in &keys {
            if crate::repo::site_api_key::decrypt(crypto, key)? == api_key {
                matched_key = Some(key.id.clone());
                break;
            }
        }
        let name_changed = existing.name != name;
        let notes_changed = notes
            .as_ref()
            .is_some_and(|incoming| existing.notes.as_ref() != Some(incoming));
        let next_caps = input
            .capabilities
            .as_ref()
            .map(|incoming| merge_codex_capabilities(&existing.capabilities, incoming));
        let caps_changed = next_caps
            .as_ref()
            .is_some_and(|caps| !capabilities_equal(caps, &existing.capabilities));
        if name_changed || notes_changed || caps_changed {
            site::update_site(
                conn,
                crypto,
                &existing.id,
                UpdateSiteInput {
                    name: Some(name.to_string()),
                    notes,
                    capabilities: next_caps,
                    ..UpdateSiteInput::default()
                },
            )?;
        }

        if matched_key.is_some() {
            let row = site::get_site(conn, &existing.id)?;
            return Ok(DeepLinkSiteImportResult {
                site: row.to_dto(),
                created: false,
                added_api_key: false,
                reused_api_key: true,
                activated_api_key: false,
            });
        }

        let label = input
            .key_name
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.to_string())
            .unwrap_or(crate::repo::site_api_key::next_label(conn, &existing.id)?);
        crate::repo::site_api_key::add(conn, crypto, &existing.id, Some(&label), api_key)?;
        let row = site::get_site(conn, &existing.id)?;
        return Ok(DeepLinkSiteImportResult {
            site: row.to_dto(),
            created: false,
            added_api_key: true,
            reused_api_key: false,
            activated_api_key: false,
        });
    }

    let row = site::create_site(
        conn,
        crypto,
        CreateSiteInput {
            name: name.to_string(),
            base_url: urls[0].clone(),
            base_urls: Some(urls),
            api_key: api_key.to_string(),
            api_key_label: input.key_name.clone(),
            extra_api_keys: Vec::new(),
            protocol: Some(protocol.as_str().to_string()),
            claude_auth_key_style: None,
            notes,
            capabilities: input
                .capabilities
                .as_ref()
                .map(|incoming| merge_codex_capabilities(&Default::default(), incoming)),
            newapi_access_token: None,
            newapi_user_id: None,
            proxy_headers: None,
        },
    )?;

    Ok(DeepLinkSiteImportResult {
        site: row.to_dto(),
        created: true,
        added_api_key: true,
        reused_api_key: false,
        activated_api_key: true,
    })
}

pub fn import_site_from_deep_link(
    state: &AppState,
    input: DeepLinkSiteImportInput,
) -> AppResult<DeepLinkSiteImportResult> {
    state
        .db
        .with_conn(|c| import_site_from_deep_link_conn(c, &state.crypto, input))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn setup() -> (Connection, Crypto) {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::apply_schema(&conn).unwrap();
        (conn, Crypto::from_key([7u8; 32]))
    }

    fn input(
        name: &str,
        urls: &[&str],
        key: &str,
        protocol: Option<&str>,
        notes: Option<&str>,
    ) -> DeepLinkSiteImportInput {
        DeepLinkSiteImportInput {
            name: name.into(),
            base_urls: urls.iter().map(|s| (*s).to_string()).collect(),
            api_key: key.into(),
            protocol: protocol.map(|s| s.into()),
            notes: notes.map(|s| s.into()),
            capabilities: None,
            key_name: None,
        }
    }

    #[test]
    fn deep_link_import_creates_site_with_multiple_routes() {
        let (conn, crypto) = setup();
        let result = import_site_from_deep_link_conn(
            &conn,
            &crypto,
            input(
                "Relay",
                &["https://a.example.com/v1", "https://b.example.com/v1"],
                "sk-example",
                Some("openai"),
                Some("hi"),
            ),
        )
        .unwrap();

        assert!(result.created);
        assert!(result.added_api_key);
        assert!(!result.reused_api_key);
        assert!(result.activated_api_key);
        assert_eq!(result.site.name, "Relay");
        assert_eq!(result.site.base_url, "https://a.example.com/v1");
        assert_eq!(
            result.site.base_urls,
            vec![
                "https://a.example.com/v1".to_string(),
                "https://b.example.com/v1".to_string()
            ]
        );
        assert_eq!(result.site.protocol, "openai_compatible");
        assert_eq!(result.site.notes.as_deref(), Some("hi"));
        assert_eq!(
            crypto
                .decrypt(
                    &site::get_site(&conn, &result.site.id)
                        .unwrap()
                        .api_key_encrypted
                )
                .unwrap(),
            "sk-example"
        );
    }

    #[test]
    fn deep_link_import_reuses_same_protocol_and_url_set() {
        let (conn, crypto) = setup();
        let first = import_site_from_deep_link_conn(
            &conn,
            &crypto,
            input(
                "First",
                &["https://b.example.com", "https://a.example.com"],
                "sk-same",
                Some("openai_compatible"),
                None,
            ),
        )
        .unwrap();
        let second = import_site_from_deep_link_conn(
            &conn,
            &crypto,
            input(
                "First",
                &["https://a.example.com", "https://b.example.com"],
                "sk-same",
                Some("openai"),
                None,
            ),
        )
        .unwrap();

        assert_eq!(second.site.id, first.site.id);
        assert!(!second.created);
        assert!(second.reused_api_key);
        assert!(!second.added_api_key);
        assert!(!second.activated_api_key);
        // Reuse must not reorder the active route.
        assert_eq!(second.site.base_url, first.site.base_url);
        assert_eq!(site::list_sites(&conn).unwrap().len(), 1);
    }

    #[test]
    fn deep_link_import_adds_inactive_key_when_urls_match() {
        let (conn, crypto) = setup();
        let first = import_site_from_deep_link_conn(
            &conn,
            &crypto,
            input("Relay", &["https://a.example.com"], "sk-old", None, None),
        )
        .unwrap();
        let second = import_site_from_deep_link_conn(
            &conn,
            &crypto,
            input(
                "Relay Two",
                &["https://a.example.com"],
                "sk-new-key",
                None,
                Some("updated"),
            ),
        )
        .unwrap();

        assert_eq!(second.site.id, first.site.id);
        assert!(!second.created);
        assert!(second.added_api_key);
        assert!(!second.reused_api_key);
        assert!(!second.activated_api_key);
        assert_eq!(second.site.name, "Relay Two");
        assert_eq!(second.site.notes.as_deref(), Some("updated"));
        assert_eq!(second.site.api_keys.len(), 2);
        assert_eq!(
            crypto
                .decrypt(
                    &site::get_site(&conn, &second.site.id)
                        .unwrap()
                        .api_key_encrypted
                )
                .unwrap(),
            "sk-old"
        );
    }

    #[test]
    fn deep_link_import_creates_when_url_set_differs() {
        let (conn, crypto) = setup();
        let first = import_site_from_deep_link_conn(
            &conn,
            &crypto,
            input("A", &["https://a.example.com"], "sk-a", None, None),
        )
        .unwrap();
        let second = import_site_from_deep_link_conn(
            &conn,
            &crypto,
            input(
                "A",
                &["https://a.example.com", "https://b.example.com"],
                "sk-a",
                None,
                None,
            ),
        )
        .unwrap();

        assert_ne!(second.site.id, first.site.id);
        assert!(second.created);
        assert_eq!(site::list_sites(&conn).unwrap().len(), 2);
    }

    #[test]
    fn deep_link_import_rejects_invalid_input() {
        let (conn, crypto) = setup();
        let cases = [
            input("", &["https://a.example.com"], "sk", None, None),
            input("N", &["ftp://a.example.com"], "sk", None, None),
            input("N", &["https://a.example.com"], "", None, None),
            input("N", &["https://a.example.com"], "sk", Some("gemini"), None),
        ];
        for case in cases {
            let err = import_site_from_deep_link_conn(&conn, &crypto, case).unwrap_err();
            let msg = err.to_string();
            assert!(!msg.is_empty(), "expected validation error");
        }
    }

    #[test]
    fn deep_link_import_stores_codex_capabilities() {
        let (conn, crypto) = setup();
        let mut first_input = input(
            "Relay",
            &["https://a.example.com"],
            "sk-example",
            None,
            None,
        );
        let mut caps = std::collections::HashMap::new();
        caps.insert("codex-compact".into(), true);
        caps.insert("codex-vision".into(), true);
        first_input.capabilities = Some(caps);
        let created = import_site_from_deep_link_conn(&conn, &crypto, first_input).unwrap();
        assert!(created.created);
        assert_eq!(created.site.capabilities.get("codex-compact"), Some(&true));
        assert_eq!(created.site.capabilities.get("codex-vision"), Some(&true));
        assert_eq!(created.site.capabilities.get("codex-search"), Some(&false));

        let mut same = input(
            "Relay",
            &["https://a.example.com"],
            "sk-example",
            None,
            None,
        );
        same.capabilities = None;
        let reused = import_site_from_deep_link_conn(&conn, &crypto, same).unwrap();
        assert!(reused.reused_api_key);
        assert_eq!(reused.site.capabilities.get("codex-compact"), Some(&true));

        let mut updated = input(
            "Relay",
            &["https://a.example.com"],
            "sk-example",
            None,
            None,
        );
        let mut next = std::collections::HashMap::new();
        next.insert("codex-search".into(), true);
        updated.capabilities = Some(next);
        let changed = import_site_from_deep_link_conn(&conn, &crypto, updated).unwrap();
        assert!(!changed.created);
        assert_eq!(changed.site.capabilities.get("codex-search"), Some(&true));
        assert_eq!(changed.site.capabilities.get("codex-compact"), Some(&false));
    }
}
