use crate::domain::{ApplyStatus, ApplyTargetResult, SiteRow, SwitchRouteResult, TargetKind};
use crate::error::AppResult;
use crate::lock::try_lock_target;
use crate::paths::backups_dir;
use crate::repo;
use crate::state::AppState;
use chrono::Utc;
use std::fs;
use std::path::PathBuf;

pub fn switch_site_route(
    state: &AppState,
    site_id: &str,
    base_url: &str,
    apply: bool,
) -> AppResult<SwitchRouteResult> {
    let _site_lock = crate::lock::try_lock_site(site_id)?;
    let before = state.db.with_conn(|c| repo::site::get_site(c, site_id))?;
    let site = state
        .db
        .with_conn(|c| repo::site::switch_site_route(c, site_id, base_url))?;
    let results = if apply && before.base_url != site.base_url {
        sync_applied_urls(state, &site)?
    } else {
        vec![]
    };
    Ok(SwitchRouteResult {
        site: site.to_dto(),
        results,
    })
}

/// 站点 base URL 变化后，把它重写进所有绑定该站点的目标配置。
///
/// 接管开启时，可写地址由 `local_proxy::routing::effective_site` 换成代理地址，
/// 于是"开关接管"和"换线路"走的是同一条重写路径。
pub fn sync_applied_urls(state: &AppState, site: &SiteRow) -> AppResult<Vec<ApplyTargetResult>> {
    let settings = state.db.with_conn(repo::settings::get_settings)?;
    let bindings = state
        .db
        .with_conn(|c| repo::binding::list_bindings_for_site(c, &site.id))?;
    let applied_at = Utc::now().timestamp_millis();
    let mut results = Vec::new();

    for binding in bindings {
        if binding.orphan {
            continue;
        }
        let target = binding.target;
        let effective = crate::local_proxy::routing::effective_site(site, target, &settings);
        results.push(run_rewrite(
            state,
            target,
            &binding,
            &effective,
            &settings,
            applied_at,
        )?);
    }

    Ok(results)
}

/// 按目标重写它的唯一绑定：用于切换接管开关后立即生效，以及退出前恢复直连。
///
/// 与 `sync_applied_urls` 的区别是"只处理一个目标"且显式传入设置
/// （退出路径上设置刚被改写，不能重新从库里读）。
pub fn sync_applied_target(
    state: &AppState,
    target: TargetKind,
    settings: &crate::domain::AppSettings,
) -> AppResult<()> {
    let Some(binding) = state.db.with_conn(|c| repo::binding::get_binding(c, target))? else {
        return Ok(());
    };
    if binding.orphan {
        return Ok(());
    }
    let Some(site_id) = binding.site_id.clone() else {
        return Ok(());
    };
    let site = state.db.with_conn(|c| repo::site::get_site(c, &site_id))?;
    let effective = crate::local_proxy::routing::effective_site(&site, target, settings);
    let applied_at = Utc::now().timestamp_millis();
    run_rewrite(state, target, &binding, &effective, settings, applied_at)?;
    Ok(())
}

/// 加锁 → 建备份目录 → 适配器 rewrite → 回写绑定 → 记录结果。
fn run_rewrite(
    state: &AppState,
    target: TargetKind,
    binding: &crate::domain::TargetBinding,
    effective_site: &SiteRow,
    settings: &crate::domain::AppSettings,
    applied_at: i64,
) -> AppResult<ApplyTargetResult> {
    let _lock = try_lock_target(target.as_str())?;
    let backup_root = backups_dir()?
        .join(target.as_str())
        .join(format!("{applied_at}"));
    fs::create_dir_all(&backup_root)?;

    let rewrite = match target {
        TargetKind::ClaudeCode => crate::adapters::claude_code::rewrite_base_url(
            effective_site,
            binding,
            settings.claude_home_override.as_deref(),
            &backup_root,
        ),
        TargetKind::Codex => crate::adapters::codex::rewrite_base_url(
            effective_site,
            binding,
            settings.codex_home_override.as_deref(),
            &backup_root,
        ),
        TargetKind::Pi => crate::adapters::pi::rewrite_base_url(
            binding,
            effective_site,
            settings.pi_agent_dir_override.as_deref(),
            &backup_root,
        ),
        TargetKind::Prime => crate::adapters::prime::rewrite_base_url(
            binding,
            effective_site,
            settings.prime_agent_dir_override.as_deref(),
            &backup_root,
        ),
    };

    let result = match rewrite {
        Ok(outcome) => {
            let mut next = binding.clone();
            next.expected_fields = outcome.expected_fields;
            next.applied_at = applied_at;
            state
                .db
                .with_conn(|c| repo::binding::upsert_binding(c, &next))?;
            ApplyTargetResult {
                target,
                ok: true,
                status: ApplyStatus::Applied,
                backup_paths: outcome.backup_paths,
                message: outcome.message,
                live_summary: Some(outcome.live_summary),
                touched_keys: None,
            }
        }
        Err(error) => ApplyTargetResult {
            target,
            ok: false,
            status: ApplyStatus::Failed,
            backup_paths: vec![],
            message: error.to_string(),
            live_summary: None,
            touched_keys: None,
        },
    };

    crate::commands::apply::finalize_backup_dir(
        &backup_root,
        target,
        &effective_site.name,
        &binding.model_id,
        None,
        applied_at,
        settings.max_backup_copies,
        &mut std::collections::HashSet::new(),
    );
    Ok(result)
}

#[allow(dead_code)]
fn unused_pathbuf(_: PathBuf) {}
