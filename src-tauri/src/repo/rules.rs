use crate::domain::{parse_persisted_targets, AgentRules, TargetKind};
use crate::error::AppResult;
use crate::repo::sync_meta;
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};

/// 已应用过的目标集合。取消勾选或清空正文时靠它把托管块从旧客户端里清掉。
pub const APPLIED_TARGETS_KEY: &str = "agent_rules_applied_targets";

/// 目标顺序固定为「应用中心」的展示顺序：同样的勾选集合永远产出同样的库内容，
/// 避免顺序差异造成无意义的数据变更与同步指纹抖动。
pub fn canonical_targets(targets: &[TargetKind]) -> Vec<TargetKind> {
    [
        TargetKind::ClaudeCode,
        TargetKind::Codex,
        TargetKind::Pi,
        TargetKind::Prime,
    ]
    .into_iter()
    .filter(|target| targets.contains(target))
    .collect()
}

/// 无行时返回「空正文 + 空目标」：空正文或空目标都表示不生效（走清理路径）。
pub fn get(conn: &Connection) -> AppResult<AgentRules> {
    let row = conn
        .query_row(
            "SELECT body, targets_json, updated_at FROM agent_rules WHERE id = 1",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )
        .optional()?;
    match row {
        Some((body, targets_json, updated_at)) => Ok(AgentRules {
            body,
            targets: canonical_targets(&parse_persisted_targets(
                &targets_json,
                "agent_rules.targets_json",
            )),
            updated_at,
        }),
        None => Ok(AgentRules::default()),
    }
}

pub fn save(conn: &Connection, body: &str, targets: &[TargetKind]) -> AppResult<AgentRules> {
    let targets = canonical_targets(targets);
    let updated_at = Utc::now().timestamp_millis();
    conn.execute(
        "INSERT INTO agent_rules (id, body, targets_json, updated_at) VALUES (1, ?1, ?2, ?3)
         ON CONFLICT(id) DO UPDATE SET
           body = excluded.body,
           targets_json = excluded.targets_json,
           updated_at = excluded.updated_at",
        params![body, serde_json::to_string(&targets)?, updated_at],
    )?;
    Ok(AgentRules {
        body: body.to_string(),
        targets,
        updated_at,
    })
}

pub fn applied_targets(conn: &Connection) -> AppResult<Vec<TargetKind>> {
    match sync_meta::get_meta(conn, APPLIED_TARGETS_KEY)? {
        Some(json) => Ok(parse_persisted_targets(&json, APPLIED_TARGETS_KEY)),
        None => Ok(Vec::new()),
    }
}

pub fn record_applied_targets(conn: &Connection, targets: &[TargetKind]) -> AppResult<()> {
    sync_meta::set_meta(
        conn,
        APPLIED_TARGETS_KEY,
        &serde_json::to_string(&canonical_targets(targets))?,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::apply_schema(&conn).unwrap();
        conn
    }

    #[test]
    fn missing_row_reads_as_empty_rules() {
        let conn = conn();
        let rules = get(&conn).unwrap();
        assert_eq!(rules.body, "");
        assert!(rules.targets.is_empty());
        assert_eq!(rules.updated_at, 0);
    }

    #[test]
    fn save_round_trips_and_dedupes_in_canonical_order() {
        let conn = conn();
        // 乱序 + 重复的目标最终按展示顺序归一，且只有一份。
        let saved = save(
            &conn,
            "# 约束\n- 说中文\n",
            &[
                TargetKind::Prime,
                TargetKind::ClaudeCode,
                TargetKind::Prime,
                TargetKind::Codex,
            ],
        )
        .unwrap();
        assert_eq!(
            saved.targets,
            vec![TargetKind::ClaudeCode, TargetKind::Codex, TargetKind::Prime]
        );

        let loaded = get(&conn).unwrap();
        assert_eq!(loaded.body, "# 约束\n- 说中文\n");
        assert_eq!(loaded.targets, saved.targets);
        assert!(loaded.updated_at > 0);
    }

    #[test]
    fn save_is_upsert_on_the_single_row() {
        let conn = conn();
        save(&conn, "first", &[TargetKind::Pi]).unwrap();
        save(&conn, "second", &[TargetKind::Prime]).unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM agent_rules", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
        let loaded = get(&conn).unwrap();
        assert_eq!(loaded.body, "second");
        assert_eq!(loaded.targets, vec![TargetKind::Prime]);
    }

    #[test]
    fn applied_targets_round_trip() {
        let conn = conn();
        assert!(applied_targets(&conn).unwrap().is_empty());
        record_applied_targets(&conn, &[TargetKind::Pi, TargetKind::ClaudeCode]).unwrap();
        assert_eq!(
            applied_targets(&conn).unwrap(),
            vec![TargetKind::ClaudeCode, TargetKind::Pi]
        );
        record_applied_targets(&conn, &[]).unwrap();
        assert!(applied_targets(&conn).unwrap().is_empty());
    }
}
