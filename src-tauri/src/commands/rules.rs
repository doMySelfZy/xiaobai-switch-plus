use crate::adapters::agent_rules as rules_adapters;
use crate::adapters::agent_rules::TargetOverrides;
use crate::domain::{
    AgentRules, AgentRulesApplyResult, AgentRulesTargetPath, AgentRulesTargetResult, AppSettings,
    TargetKind,
};
use crate::error::AppResult;
use crate::paths::backups_dir;
use crate::repo;
use crate::state::AppState;
use tauri::State;

fn overrides_from(settings: &AppSettings) -> TargetOverrides {
    TargetOverrides {
        claude_home: settings.claude_home_override.clone(),
        codex_home: settings.codex_home_override.clone(),
        pi_agent_dir: settings.pi_agent_dir_override.clone(),
        prime_agent_dir: settings.prime_agent_dir_override.clone(),
    }
}

#[tauri::command]
pub fn get_agent_rules(state: State<'_, AppState>) -> AppResult<AgentRules> {
    state.db.with_conn(repo::rules::get)
}

/// 需要写盘的目标 = 本次勾选的目标 ∪ 上次写过的目标。
///
/// 并上「上次写过的目标」是因为用户取消勾选、清空正文后，旧客户端文件里的托管块
/// 必须被摘掉，否则会留下孤儿内容。清空正文时目标集合为空，完全依赖这个并集来清理。
fn merged_targets(requested: &[TargetKind], previously_applied: &[TargetKind]) -> Vec<TargetKind> {
    let mut union = requested.to_vec();
    for target in previously_applied {
        if !union.contains(target) {
            union.push(*target);
        }
    }
    union
}

/// 应用后要记下的目标 = 「仍然有约束指向的目标」∪「本次写失败的目标」。
///
/// 被清理干净的目标不该继续留在记录里，否则以后每次保存都会重写它那份客户端文件；
/// 写失败的目标要留下，下次仍然参与清理重试。
fn targets_to_record(
    results: &[AgentRulesTargetResult],
    desired: &[TargetKind],
) -> Vec<TargetKind> {
    let mut recorded: Vec<TargetKind> = Vec::new();
    for result in results {
        let keep = !result.ok || desired.contains(&result.target);
        if keep && !recorded.contains(&result.target) {
            recorded.push(result.target);
        }
    }
    recorded
}

/// 把约束写进（或从）目标客户端文件。
fn apply_to_targets(
    state: &AppState,
    body: &str,
    targets: &[TargetKind],
) -> AppResult<AgentRulesApplyResult> {
    let settings: AppSettings = state.db.with_conn(repo::settings::get_settings)?;
    let overrides = overrides_from(&settings);
    let previously_applied = state.db.with_conn(repo::rules::applied_targets)?;
    let union = merged_targets(targets, &previously_applied);

    // 空正文表示「不生效」：清掉所有托管块，而不是写一个空块进去。
    let effective: Option<&str> = if body.trim().is_empty() {
        None
    } else {
        Some(body)
    };

    let backup_root = backups_dir()?.join("agent-rules");
    std::fs::create_dir_all(&backup_root)?;

    let mut results = Vec::new();
    for target in &union {
        let outcome =
            rules_adapters::apply_to_target(*target, effective, &overrides, &backup_root);
        results.push(match outcome {
            Ok(result) => result,
            Err(error) => AgentRulesTargetResult {
                target: *target,
                ok: false,
                path: String::new(),
                changed: false,
                backup_paths: Vec::new(),
                message: error.to_string(),
            },
        });
    }

    let desired: Vec<TargetKind> = if effective.is_some() {
        targets.to_vec()
    } else {
        Vec::new()
    };
    let recorded = targets_to_record(&results, &desired);
    state
        .db
        .with_conn(|conn| repo::rules::record_applied_targets(conn, &recorded))?;

    Ok(AgentRulesApplyResult {
        results,
        applied_at: chrono::Utc::now().timestamp_millis(),
    })
}

/// 保存约束并立即写入所选目标（与 MCP 的保存即应用一致，不需要再点一次应用）。
#[tauri::command]
pub fn save_agent_rules(
    state: State<'_, AppState>,
    body: String,
    targets: Vec<TargetKind>,
) -> AppResult<AgentRulesApplyResult> {
    // 标记字面量会导致托管块无法可靠界定，必须在落库前拦下。
    rules_adapters::validate_body_for_storage(&body)?;
    let saved = state
        .db
        .with_conn(|conn| repo::rules::save(conn, &body, &targets))?;
    apply_to_targets(&state, &saved.body, &saved.targets)
}

/// 供界面展示四个目标的实际落点与状态；Codex 的遮蔽文件在这一并回报。
#[tauri::command]
pub fn agent_rules_target_paths(state: State<'_, AppState>) -> AppResult<Vec<AgentRulesTargetPath>> {
    let settings: AppSettings = state.db.with_conn(repo::settings::get_settings)?;
    let overrides = overrides_from(&settings);
    let shadow = rules_adapters::codex_shadow(&overrides)?;

    Ok(rules_adapters::target_paths(&overrides)?
        .into_iter()
        .map(|(target, path)| AgentRulesTargetPath {
            target,
            exists: path.is_file(),
            path: path.display().to_string(),
            shadowed_by: if target == TargetKind::Codex {
                shadow.clone()
            } else {
                None
            },
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(target: TargetKind, ok: bool) -> AgentRulesTargetResult {
        AgentRulesTargetResult {
            target,
            ok,
            path: "/tmp/x".into(),
            changed: ok,
            backup_paths: Vec::new(),
            message: String::new(),
        }
    }

    #[test]
    fn merged_targets_unions_requested_and_previous() {
        let union = merged_targets(&[TargetKind::Codex], &[TargetKind::ClaudeCode]);
        assert_eq!(
            union,
            vec![TargetKind::Codex, TargetKind::ClaudeCode],
            "previous target must still be visited so its block gets removed"
        );
    }

    #[test]
    fn merged_targets_dedupes() {
        let union = merged_targets(&[TargetKind::Pi], &[TargetKind::Pi, TargetKind::Prime]);
        assert_eq!(union, vec![TargetKind::Pi, TargetKind::Prime]);
    }

    #[test]
    fn merged_targets_is_empty_on_full_clear() {
        assert!(merged_targets(&[], &[]).is_empty());
    }

    #[test]
    fn targets_to_record_drops_cleared_targets() {
        // 目标被清理干净（desired 里没有它）后不该继续记录，否则每次保存都会重写该文件。
        let recorded = targets_to_record(
            &[
                result(TargetKind::ClaudeCode, true),
                result(TargetKind::Codex, true),
            ],
            &[TargetKind::Codex],
        );
        assert_eq!(recorded, vec![TargetKind::Codex]);
    }

    #[test]
    fn targets_to_record_keeps_failed_targets_for_retry() {
        let recorded = targets_to_record(
            &[
                result(TargetKind::ClaudeCode, false),
                result(TargetKind::Codex, true),
            ],
            &[TargetKind::Codex],
        );
        assert_eq!(recorded, vec![TargetKind::ClaudeCode, TargetKind::Codex]);
    }
}
