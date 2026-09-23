use crate::crypto::Crypto;
use crate::domain::{McpKind, McpServer, McpServerInput, McpServerSummary};
use crate::error::{AppError, AppResult};
use crate::repo::sync_meta;
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::json;
use uuid::Uuid;

/// 已应用过的目标集合。删除或改绑目标时靠它把托管条目从旧客户端里清掉。
const APPLIED_TARGETS_KEY: &str = "mcp_applied_targets";

const COLUMNS: &str =
    "id, name, kind, enabled, targets_json, config_json, secrets_encrypted, created_at, updated_at, current_version, latest_version, last_update_check_at";

fn kind_from_str(value: &str) -> McpKind {
    match value {
        "sse" => McpKind::Sse,
        "http" => McpKind::Http,
        _ => McpKind::Stdio,
    }
}

fn kind_to_str(kind: McpKind) -> &'static str {
    match kind {
        McpKind::Stdio => "stdio",
        McpKind::Sse => "sse",
        McpKind::Http => "http",
    }
}

fn read_row(row: &rusqlite::Row<'_>, crypto: &Crypto) -> AppResult<McpServer> {
    let id: String = row.get(0)?;
    let name: String = row.get(1)?;
    let kind: String = row.get(2)?;
    let enabled: i64 = row.get(3)?;
    let targets_json: String = row.get(4)?;
    let config_json: String = row.get(5)?;
    let secrets: Option<String> = row.get(6)?;
    let created_at: i64 = row.get(7)?;
    let updated_at: i64 = row.get(8)?;
    let current_version: Option<String> = row.get(9)?;
    let latest_version: Option<String> = row.get(10)?;
    let last_update_check_at: Option<i64> = row.get(11)?;

    let secrets: serde_json::Value = match secrets {
        Some(encoded) => serde_json::from_str(&crypto.decrypt(&encoded)?)?,
        None => json!({}),
    };
    Ok(McpServer {
        id,
        name,
        kind: kind_from_str(&kind),
        enabled: enabled != 0,
        targets: serde_json::from_str(&targets_json)?,
        config: serde_json::from_str(&config_json)?,
        env: secrets.get("env").cloned().unwrap_or_else(|| json!({})),
        headers: secrets
            .get("headers")
            .cloned()
            .unwrap_or_else(|| json!({})),
        created_at,
        updated_at,
        current_version,
        latest_version,
        last_update_check_at,
    })
}

pub fn list(conn: &Connection, crypto: &Crypto) -> AppResult<Vec<McpServerSummary>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM mcp_servers ORDER BY name COLLATE NOCASE, id"
    ))?;
    let mut rows = stmt.query([])?;
    let mut servers = Vec::new();
    while let Some(row) = rows.next()? {
        let server = read_row(row, crypto)?;
        let absolute_command =
            crate::adapters::mcp_identity::command_is_absolute(server.kind, &server.config);
        servers.push(McpServerSummary {
            id: server.id,
            name: server.name,
            kind: server.kind,
            enabled: server.enabled,
            targets: server.targets,
            created_at: server.created_at,
            updated_at: server.updated_at,
            current_version: server.current_version,
            latest_version: server.latest_version,
            last_update_check_at: server.last_update_check_at,
            absolute_command,
        });
    }
    Ok(servers)
}

pub fn list_full(conn: &Connection, crypto: &Crypto) -> AppResult<Vec<McpServer>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM mcp_servers ORDER BY name COLLATE NOCASE, id"
    ))?;
    let mut rows = stmt.query([])?;
    let mut servers = Vec::new();
    while let Some(row) = rows.next()? {
        servers.push(read_row(row, crypto)?);
    }
    Ok(servers)
}

pub fn get(conn: &Connection, id: &str, crypto: &Crypto) -> AppResult<McpServer> {
    let mut stmt = conn.prepare(&format!("SELECT {COLUMNS} FROM mcp_servers WHERE id = ?1"))?;
    let mut rows = stmt.query(params![id])?;
    match rows.next()? {
        Some(row) => read_row(row, crypto),
        None => Err(AppError::new("not_found", "MCP server not found")),
    }
}

fn validate(input: &McpServerInput) -> AppResult<String> {
    let name = input.name.trim();
    if name.is_empty() {
        return Err(AppError::new(
            "validation_failed",
            "MCP server name is required",
        ));
    }
    // 名称会拼进托管键（mcpServers / [mcp_servers]），限制字符集避免写出非法键名。
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(AppError::new(
            "validation_failed",
            "MCP server name may only contain letters, numbers, '_' and '-'",
        ));
    }
    if !input.config.is_object() {
        return Err(AppError::new(
            "validation_failed",
            "MCP config must be a JSON object",
        ));
    }
    if !input.env.is_object() || !input.headers.is_object() {
        return Err(AppError::new(
            "validation_failed",
            "MCP environment and headers must be JSON objects",
        ));
    }
    Ok(name.to_string())
}

pub fn save(conn: &Connection, crypto: &Crypto, input: McpServerInput) -> AppResult<McpServer> {
    let name = validate(&input)?;
    let id = input.id.unwrap_or_else(|| Uuid::new_v4().to_string());
    let existing: Option<String> = conn
        .query_row(
            "SELECT name FROM mcp_servers WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )
        .optional()?;
    let duplicate: Option<String> = conn
        .query_row(
            "SELECT id FROM mcp_servers WHERE name = ?1 COLLATE NOCASE AND id <> ?2",
            params![name, id],
            |row| row.get(0),
        )
        .optional()?;
    if duplicate.is_some() {
        return Err(AppError::new(
            "validation_failed",
            format!("another MCP server is already named '{name}'"),
        ));
    }

    let now = Utc::now().timestamp_millis();
    let created_at = match existing {
        Some(_) => conn.query_row(
            "SELECT created_at FROM mcp_servers WHERE id = ?1",
            params![id],
            |row| row.get::<_, i64>(0),
        )?,
        None => now,
    };
    // env / headers 只在 secrets 列里存放，且始终加密。
    let secrets =
        crypto.encrypt(&json!({ "env": input.env, "headers": input.headers }).to_string())?;

    conn.execute(
        "INSERT INTO mcp_servers (id, name, kind, enabled, targets_json, config_json, secrets_encrypted, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)
         ON CONFLICT(id) DO UPDATE SET
           name = excluded.name,
           kind = excluded.kind,
           enabled = excluded.enabled,
           targets_json = excluded.targets_json,
           config_json = excluded.config_json,
           secrets_encrypted = excluded.secrets_encrypted,
           updated_at = excluded.updated_at",
        params![
            id,
            name,
            kind_to_str(input.kind),
            input.enabled as i64,
            serde_json::to_string(&input.targets)?,
            serde_json::to_string(&input.config)?,
            secrets,
            created_at,
        ],
    )?;
    get(conn, &id, crypto)
}

pub fn delete(conn: &Connection, id: &str) -> AppResult<()> {
    let removed = conn.execute("DELETE FROM mcp_servers WHERE id = ?1", params![id])?;
    if removed == 0 {
        return Err(AppError::new("not_found", "MCP server not found"));
    }
    Ok(())
}

pub fn applied_targets(conn: &Connection) -> AppResult<Vec<crate::domain::TargetKind>> {
    match sync_meta::get_meta(conn, APPLIED_TARGETS_KEY)? {
        Some(raw) => Ok(crate::domain::parse_persisted_targets(&raw, APPLIED_TARGETS_KEY)),
        None => Ok(Vec::new()),
    }
}

pub fn record_applied_targets(
    conn: &Connection,
    targets: &[crate::domain::TargetKind],
) -> AppResult<()> {
    sync_meta::set_meta(conn, APPLIED_TARGETS_KEY, &serde_json::to_string(targets)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::TargetKind;
    use serde_json::json;

    const FIXTURE_ENV_KEY: &str = "DEMO_VAR";
    const FIXTURE_ENV_VALUE: &str = "placeholder-not-a-secret";

    fn crypto() -> Crypto {
        Crypto::from_key([9_u8; 32])
    }

    fn conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::apply_schema(&conn).unwrap();
        conn
    }

    fn input(name: &str) -> McpServerInput {
        McpServerInput {
            id: None,
            name: name.to_string(),
            kind: McpKind::Stdio,
            enabled: true,
            targets: vec![TargetKind::ClaudeCode],
            config: json!({"command": "mcp-demo"}),
            env: json!({ FIXTURE_ENV_KEY: FIXTURE_ENV_VALUE }),
            headers: json!({}),
        }
    }

    #[test]
    fn round_trip_encrypts_env_and_keeps_config_readable() {
        let conn = conn();
        let crypto = crypto();
        let saved = save(&conn, &crypto, input("demo")).unwrap();

        let stored: (String, String) = conn
            .query_row(
                "SELECT config_json, secrets_encrypted FROM mcp_servers WHERE id = ?1",
                params![saved.id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert!(stored.0.contains("mcp-demo"));
        assert!(
            !stored.1.contains(FIXTURE_ENV_VALUE),
            "env values must not be stored in plaintext"
        );

        let loaded = get(&conn, &saved.id, &crypto).unwrap();
        assert_eq!(loaded.env[FIXTURE_ENV_KEY], FIXTURE_ENV_VALUE);
        assert_eq!(loaded.config["command"], "mcp-demo");
    }

    #[test]
    fn save_updates_in_place_and_preserves_created_at() {
        let conn = conn();
        let crypto = crypto();
        let saved = save(&conn, &crypto, input("demo")).unwrap();
        let mut update = input("demo");
        update.id = Some(saved.id.clone());
        update.enabled = false;
        update.targets = vec![TargetKind::Codex];
        let updated = save(&conn, &crypto, update).unwrap();

        assert_eq!(updated.id, saved.id);
        assert!(!updated.enabled);
        assert_eq!(updated.targets, vec![TargetKind::Codex]);
        assert_eq!(updated.created_at, saved.created_at);
        assert_eq!(list(&conn, &crypto).unwrap().len(), 1);
    }

    #[test]
    fn rejects_invalid_and_duplicate_names() {
        let conn = conn();
        let crypto = crypto();
        let mut bad = input("has space");
        assert!(save(&conn, &crypto, bad.clone()).is_err());
        bad.name = String::new();
        assert!(save(&conn, &crypto, bad).is_err());

        save(&conn, &crypto, input("demo")).unwrap();
        let error = save(&conn, &crypto, input("DEMO")).unwrap_err();
        assert!(error.to_string().contains("already named"));
    }

    #[test]
    fn delete_reports_missing_rows() {
        let conn = conn();
        let crypto = crypto();
        let saved = save(&conn, &crypto, input("demo")).unwrap();
        delete(&conn, &saved.id).unwrap();
        assert!(list(&conn, &crypto).unwrap().is_empty());
        assert!(delete(&conn, &saved.id).is_err());
    }

    #[test]
    fn applied_targets_round_trip() {
        let conn = conn();
        assert!(applied_targets(&conn).unwrap().is_empty());
        record_applied_targets(&conn, &[TargetKind::ClaudeCode, TargetKind::Prime]).unwrap();
        assert_eq!(
            applied_targets(&conn).unwrap(),
            vec![TargetKind::ClaudeCode, TargetKind::Prime]
        );
    }
}
