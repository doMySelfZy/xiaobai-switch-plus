use crate::backup::{self, BackupMeta, BackupMetaFile};
use crate::capabilities::{
    capability_on, CODEX_COMPACT, CODEX_IMAGEGEN, CODEX_SEARCH, CODEX_VISION,
};
use crate::domain::{
    ApplyRecordDto, ApplyResult, ApplyStatus, ApplyTargetResult, BackupInfo, BackupPreview,
    CapabilitySource, ClaudeApplyOptions, ClaudeAuthKeyStyle, ClaudeEffortLevel, CodexApplyOptions,
    CodexReasoningEffort, PiApplyOptions, PrimeApplyOptions, TargetKind, TouchedKeys,
};
use crate::error::{AppError, AppResult};
use crate::lock::try_lock_target;
use crate::paths::backups_dir;
use crate::repo;
use crate::state::AppState;
use chrono::Utc;
use std::fs;
use std::path::Path;
use tauri::State;
use uuid::Uuid;

fn non_empty(s: Option<String>) -> Option<String> {
    s.map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

/// `(async)` 是必须的：普通 `#[tauri::command]` 的同步函数在 IPC 处理线程上
/// 直接执行（`tauri-macros` 生成 `kind.block(result, resolver)`，不跳线程池），
/// 而本命令会做文件写入、`fsync`、备份与剪枝，整段时间会卡住窗口消息泵。
/// 加 `(async)` 后主体在异步运行时上执行，`try_lock_*` 目标级锁语义不变。
#[tauri::command(async)]
pub fn apply_site(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    site_id: String,
    targets: Vec<TargetKind>,
    model_id: String,
    claude_auth_key_style: Option<String>,
    claude_fable_model_id: Option<String>,
    claude_opus_model_id: Option<String>,
    claude_sonnet_model_id: Option<String>,
    claude_haiku_model_id: Option<String>,
    claude_effort_level: Option<String>,
    claude_use_1m_context: Option<bool>,
    codex_write_all_models: Option<bool>,
    codex_reasoning_effort: Option<String>,
    codex_remote_compaction: Option<bool>,
    codex_image_understanding: Option<bool>,
    codex_image_generation: Option<bool>,
    codex_web_search: Option<bool>,
    codex_capability_source: Option<String>,
    pi_write_all_models: Option<bool>,
    prime_write_all_models: Option<bool>,
    api_key_id: Option<String>,
) -> AppResult<ApplyResult> {
    if targets.is_empty() {
        return Err(AppError::new("validation_failed", "no targets selected"));
    }
    if model_id.trim().is_empty() {
        return Err(AppError::new("validation_failed", "model id required"));
    }

    let _site_lock = crate::lock::try_lock_site(&site_id)?;
    let settings = state.db.with_conn(repo::settings::get_settings)?;
    let (site, api_key) = state.db.with_conn(|c| {
        let site = repo::site::get_site(c, &site_id)?;
        let key = match api_key_id.as_deref() {
            Some(id) => repo::site_api_key::require_active(c, &site_id, id)?,
            None => repo::site_api_key::get_active(c, &site_id)?,
        };
        let secret = repo::site_api_key::decrypt(&state.crypto, &key)?;
        Ok((site, secret))
    })?;
    let key_snapshot = site.api_key_snapshot();

    let auth = claude_auth_key_style
        .as_deref()
        .map(ClaudeAuthKeyStyle::parse)
        .unwrap_or(site.claude_auth_key_style.clone());

    let claude_opts = ClaudeApplyOptions {
        fable_model_id: non_empty(claude_fable_model_id),
        opus_model_id: non_empty(claude_opus_model_id),
        sonnet_model_id: non_empty(claude_sonnet_model_id),
        haiku_model_id: non_empty(claude_haiku_model_id),
        effort_level: claude_effort_level
            .as_deref()
            .and_then(ClaudeEffortLevel::parse),
        use_1m_context: claude_use_1m_context.unwrap_or(false),
    };

    let capability_source = CapabilitySource::parse(codex_capability_source.as_deref());
    let (remote_compaction, image_understanding, image_generation, web_search) =
        match capability_source {
            CapabilitySource::Site => (
                capability_on(&site.capabilities, CODEX_COMPACT),
                capability_on(&site.capabilities, CODEX_VISION),
                capability_on(&site.capabilities, CODEX_IMAGEGEN),
                capability_on(&site.capabilities, CODEX_SEARCH),
            ),
            CapabilitySource::Custom => (
                codex_remote_compaction.unwrap_or(false),
                codex_image_understanding.unwrap_or(false),
                codex_image_generation.unwrap_or(false),
                codex_web_search.unwrap_or(false),
            ),
        };

    let write_all = codex_write_all_models.unwrap_or(false);
    let catalog_models = if write_all && targets.contains(&TargetKind::Codex) {
        state.db.with_conn(|c| {
            let models = repo::site::list_models(c, &site_id)?;
            Ok(models
                .into_iter()
                .map(|m| (m.model_id, m.display_name))
                .collect::<Vec<_>>())
        })?
    } else {
        vec![]
    };

    let codex_opts = CodexApplyOptions {
        write_all_models: write_all,
        reasoning_effort: codex_reasoning_effort
            .as_deref()
            .and_then(CodexReasoningEffort::parse),
        catalog_models,
        remote_compaction,
        image_understanding,
        image_generation,
        web_search,
        capability_source,
    };

    let pi_write_all = pi_write_all_models.unwrap_or(false);
    let pi_catalog_models = if pi_write_all && targets.contains(&TargetKind::Pi) {
        state.db.with_conn(|c| {
            let models = repo::site::list_models(c, &site_id)?;
            Ok(models
                .into_iter()
                .map(|model| (model.model_id, model.display_name))
                .collect::<Vec<_>>())
        })?
    } else {
        Vec::new()
    };
    let mut pi_thinking = if targets.contains(&TargetKind::Pi) {
        state
            .db
            .with_conn(|c| repo::thinking::get_write(c, &site_id, TargetKind::Pi))?
    } else {
        crate::domain::ThinkingWrite::default()
    };
    crate::adapters::thinking::fill_extended_maps(&mut pi_thinking);
    let pi_opts = PiApplyOptions {
        write_all_models: pi_write_all,
        catalog_models: pi_catalog_models,
        thinking: pi_thinking,
    };

    let prime_write_all = prime_write_all_models.unwrap_or(false);
    let prime_catalog_models = if prime_write_all && targets.contains(&TargetKind::Prime) {
        state.db.with_conn(|c| {
            let models = repo::site::list_models(c, &site_id)?;
            Ok(models
                .into_iter()
                .map(|model| (model.model_id, model.display_name))
                .collect::<Vec<_>>())
        })?
    } else {
        Vec::new()
    };
    let mut prime_thinking = if targets.contains(&TargetKind::Prime) {
        state
            .db
            .with_conn(|c| repo::thinking::get_write(c, &site_id, TargetKind::Prime))?
    } else {
        crate::domain::ThinkingWrite::default()
    };
    crate::adapters::thinking::fill_extended_maps(&mut prime_thinking);
    let prime_opts = PrimeApplyOptions {
        write_all_models: prime_write_all,
        catalog_models: prime_catalog_models,
        thinking: prime_thinking,
    };

    let applied_at = Utc::now().timestamp_millis();
    let mut results = Vec::new();
    let mut targets_to_prune = std::collections::HashSet::new();

    for target in targets {
        let _lock = try_lock_target(target.as_str())?;
        let backup_root = backups_dir()?
            .join(target.as_str())
            .join(format!("{}", applied_at));
        fs::create_dir_all(&backup_root)?;

        let binding_before = state
            .db
            .with_conn(|c| repo::binding::get_binding(c, target))?;

        // 接管开启时本站点的可写地址换成代理地址；数据库里始终保留真实上游。
        let effective = crate::local_proxy::routing::effective_site(&site, target, &settings);

        if target == TargetKind::Pi {
            match crate::adapters::pi::apply(
                &effective,
                &api_key,
                &model_id,
                &pi_opts,
                binding_before.as_ref(),
                settings.pi_agent_dir_override.as_deref(),
                &backup_root,
            ) {
                Ok(outcome) => {
                    let record_id = outcome
                        .binding
                        .apply_record_id
                        .clone()
                        .unwrap_or_else(|| Uuid::new_v4().to_string());
                    let mut binding = outcome.binding.clone();
                    crate::key_switch::stamp_binding(&mut binding, &key_snapshot);
                    binding.apply_record_id = Some(record_id.clone());
                    state
                        .db
                        .with_conn(|c| repo::binding::upsert_binding(c, &binding))?;
                    state.db.with_conn(|c| {
                        repo::apply::insert_record(
                            c,
                            &record_id,
                            Some(&site.id),
                            &site.name,
                            target.as_str(),
                            &model_id,
                            Some(&outcome.provider_id),
                            "success",
                            Some(&backup_root.display().to_string()),
                            &outcome.touched,
                            None,
                            applied_at,
                        )
                    })?;
                    results.push(ApplyTargetResult {
                        target,
                        ok: true,
                        status: ApplyStatus::Applied,
                        backup_paths: outcome.backup_paths,
                        message: outcome.message,
                        live_summary: Some(outcome.live_summary),
                        touched_keys: Some(outcome.touched.paths),
                    });
                }
                Err(error) => {
                    let touched = TouchedKeys::default();
                    let _ = state.db.with_conn(|c| {
                        repo::apply::insert_record(
                            c,
                            &Uuid::new_v4().to_string(),
                            Some(&site.id),
                            &site.name,
                            target.as_str(),
                            &model_id,
                            None,
                            "failed",
                            Some(&backup_root.display().to_string()),
                            &touched,
                            Some(&error.to_string()),
                            applied_at,
                        )
                    });
                    results.push(ApplyTargetResult {
                        target,
                        ok: false,
                        status: ApplyStatus::Failed,
                        backup_paths: Vec::new(),
                        message: error.to_string(),
                        live_summary: None,
                        touched_keys: None,
                    });
                }
            }
            finalize_backup_dir(
                &backup_root,
                target,
                &site.name,
                &model_id,
                None,
                applied_at,
                settings.max_backup_copies,
                &mut targets_to_prune,
            );
            continue;
        }

        if target == TargetKind::Prime {
            match crate::adapters::prime::apply(
                &effective,
                &api_key,
                &model_id,
                &prime_opts,
                binding_before.as_ref(),
                settings.prime_agent_dir_override.as_deref(),
                &backup_root,
            ) {
                Ok(outcome) => {
                    let record_id = outcome
                        .binding
                        .apply_record_id
                        .clone()
                        .unwrap_or_else(|| Uuid::new_v4().to_string());
                    let mut binding = outcome.binding.clone();
                    crate::key_switch::stamp_binding(&mut binding, &key_snapshot);
                    binding.apply_record_id = Some(record_id.clone());
                    state
                        .db
                        .with_conn(|c| repo::binding::upsert_binding(c, &binding))?;
                    state.db.with_conn(|c| {
                        repo::apply::insert_record(
                            c,
                            &record_id,
                            Some(&site.id),
                            &site.name,
                            target.as_str(),
                            &model_id,
                            Some(&outcome.provider_id),
                            "success",
                            Some(&backup_root.display().to_string()),
                            &outcome.touched,
                            None,
                            applied_at,
                        )
                    })?;
                    results.push(ApplyTargetResult {
                        target,
                        ok: true,
                        status: ApplyStatus::Applied,
                        backup_paths: outcome.backup_paths,
                        message: outcome.message,
                        live_summary: Some(outcome.live_summary),
                        touched_keys: Some(outcome.touched.paths),
                    });
                }
                Err(error) => {
                    let touched = TouchedKeys::default();
                    let _ = state.db.with_conn(|c| {
                        repo::apply::insert_record(
                            c,
                            &Uuid::new_v4().to_string(),
                            Some(&site.id),
                            &site.name,
                            target.as_str(),
                            &model_id,
                            None,
                            "failed",
                            Some(&backup_root.display().to_string()),
                            &touched,
                            Some(&error.to_string()),
                            applied_at,
                        )
                    });
                    results.push(ApplyTargetResult {
                        target,
                        ok: false,
                        status: ApplyStatus::Failed,
                        backup_paths: Vec::new(),
                        message: error.to_string(),
                        live_summary: None,
                        touched_keys: None,
                    });
                }
            }
            finalize_backup_dir(
                &backup_root,
                target,
                &site.name,
                &model_id,
                None,
                applied_at,
                settings.max_backup_copies,
                &mut targets_to_prune,
            );
            continue;
        }

        if target == TargetKind::Codex {
            match crate::adapters::codex::apply(
                &effective,
                &api_key,
                &model_id,
                &codex_opts,
                settings.codex_home_override.as_deref(),
                &backup_root,
            ) {
                Ok(o) => {
                    let inject_msg =
                        crate::env_inject::inject_codex_env(&settings, &o.env_key, &api_key)
                            .unwrap_or_else(|e| e.to_string());
                    let record_id = o
                        .binding
                        .apply_record_id
                        .clone()
                        .unwrap_or_else(|| Uuid::new_v4().to_string());
                    let mut binding = o.binding.clone();
                    crate::key_switch::stamp_binding(&mut binding, &key_snapshot);
                    binding.apply_record_id = Some(record_id.clone());
                    state
                        .db
                        .with_conn(|c| repo::binding::upsert_binding(c, &binding))?;
                    state.db.with_conn(|c| {
                        repo::apply::insert_record(
                            c,
                            &record_id,
                            Some(&site.id),
                            &site.name,
                            target.as_str(),
                            &model_id,
                            Some(&o.provider_id),
                            "success",
                            Some(&backup_root.display().to_string()),
                            &o.touched,
                            None,
                            applied_at,
                        )
                    })?;
                    results.push(ApplyTargetResult {
                        target,
                        ok: true,
                        status: ApplyStatus::Applied,
                        backup_paths: o.backup_paths,
                        message: format!("{} {}", o.message, inject_msg),
                        live_summary: Some(o.live_summary),
                        touched_keys: Some(o.touched.env_keys),
                    });
                }
                Err(e) => {
                    let touched = TouchedKeys::default();
                    let _ = state.db.with_conn(|c| {
                        repo::apply::insert_record(
                            c,
                            &Uuid::new_v4().to_string(),
                            Some(&site.id),
                            &site.name,
                            target.as_str(),
                            &model_id,
                            None,
                            "failed",
                            Some(&backup_root.display().to_string()),
                            &touched,
                            Some(&e.to_string()),
                            applied_at,
                        )
                    });
                    results.push(ApplyTargetResult {
                        target,
                        ok: false,
                        status: ApplyStatus::Failed,
                        backup_paths: vec![],
                        message: e.to_string(),
                        live_summary: None,
                        touched_keys: None,
                    });
                }
            }
            finalize_backup_dir(
                &backup_root,
                target,
                &site.name,
                &model_id,
                None,
                applied_at,
                settings.max_backup_copies,
                &mut targets_to_prune,
            );
            continue;
        }

        // Claude Code
        match crate::adapters::claude_code::apply(
            &effective,
            &api_key,
            &model_id,
            auth.clone(),
            settings.force_exclusive_claude_auth_key,
            &claude_opts,
            binding_before.as_ref(),
            settings.claude_home_override.as_deref(),
            &backup_root,
        ) {
            Ok(o) => {
                let record_id = o
                    .binding
                    .apply_record_id
                    .clone()
                    .unwrap_or_else(|| Uuid::new_v4().to_string());
                let mut binding = o.binding.clone();
                binding.apply_record_id = Some(record_id.clone());
                state
                    .db
                    .with_conn(|c| repo::binding::upsert_binding(c, &binding))?;
                state.db.with_conn(|c| {
                    repo::apply::insert_record(
                        c,
                        &record_id,
                        Some(&site.id),
                        &site.name,
                        target.as_str(),
                        &model_id,
                        None,
                        "success",
                        Some(&backup_root.display().to_string()),
                        &o.touched,
                        None,
                        applied_at,
                    )
                })?;
                results.push(ApplyTargetResult {
                    target,
                    ok: true,
                    status: ApplyStatus::Applied,
                    backup_paths: o.backup_paths,
                    message: o.message,
                    live_summary: Some(o.live_summary),
                    touched_keys: Some(o.touched.claude_env_keys),
                });
            }
            Err(e) => {
                let touched = TouchedKeys::default();
                let _ = state.db.with_conn(|c| {
                    repo::apply::insert_record(
                        c,
                        &Uuid::new_v4().to_string(),
                        Some(&site.id),
                        &site.name,
                        target.as_str(),
                        &model_id,
                        None,
                        "failed",
                        Some(&backup_root.display().to_string()),
                        &touched,
                        Some(&e.to_string()),
                        applied_at,
                    )
                });
                results.push(ApplyTargetResult {
                    target,
                    ok: false,
                    status: ApplyStatus::Failed,
                    backup_paths: vec![],
                    message: e.to_string(),
                    live_summary: None,
                    touched_keys: None,
                });
            }
        }
        finalize_backup_dir(
            &backup_root,
            target,
            &site.name,
            &model_id,
            None,
            applied_at,
            settings.max_backup_copies,
            &mut targets_to_prune,
        );
    }

    // 批量剪枝：所有目标应用完成后，对每个目标只扫描一次备份目录
    for target in targets_to_prune {
        let _ = backup::prune_target_backups(target, settings.max_backup_copies);
    }

    let result = ApplyResult {
        site_id,
        model_id,
        results,
        applied_at,
    };
    crate::tray::request_tray_menu_sync(&app);
    Ok(result)
}

pub(crate) fn finalize_backup_dir(
    backup_root: &Path,
    target: TargetKind,
    site_name: &str,
    model_id: &str,
    apply_record_id: Option<&str>,
    created_at: i64,
    max_copies: u32,
    targets_to_prune: &mut std::collections::HashSet<TargetKind>,
) {
    let files = backup::payload_files(backup_root);
    if files.is_empty() {
        let _ = fs::remove_dir_all(backup_root);
    } else {
        let origins = crate::adapters::atomic::read_origins(backup_root);
        let prev = backup::read_meta(backup_root);
        let prev_map: std::collections::HashMap<String, Option<String>> = prev
            .map(|m| {
                m.files
                    .into_iter()
                    .map(|f| (f.name, f.original_path))
                    .collect()
            })
            .unwrap_or_default();
        let meta = BackupMeta {
            version: 1,
            target: target.as_str().into(),
            created_at,
            site_name: Some(site_name.into()),
            model_id: Some(model_id.into()),
            apply_record_id: apply_record_id.map(|s| s.to_string()),
            files: files
                .into_iter()
                .map(|name| BackupMetaFile {
                    original_path: origins
                        .get(&name)
                        .cloned()
                        .or_else(|| prev_map.get(&name).cloned().flatten()),
                    name,
                })
                .collect(),
        };
        let _ = backup::write_meta(backup_root, &meta);
    }
    // 收集需要剪枝的目标，避免在循环中重复扫描备份目录
    targets_to_prune.insert(target);
}

/// 见 `apply_site`：改目标配置文件 + 备份清理，必须离开 IPC 线程。
#[tauri::command(async)]
pub fn revert_target(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    target: TargetKind,
) -> AppResult<()> {
    let _lock = try_lock_target(target.as_str())?;
    let settings = state.db.with_conn(repo::settings::get_settings)?;
    let binding = state
        .db
        .with_conn(|c| repo::binding::get_binding(c, target))?
        .ok_or_else(|| AppError::new("not_found", "no binding to revert"))?;

    let mut targets_to_prune: std::collections::HashSet<TargetKind> = std::collections::HashSet::new();

    match target {
        TargetKind::ClaudeCode => {
            crate::adapters::claude_code::surgical_revert(
                &binding,
                settings.claude_home_override.as_deref(),
            )?;
        }
        TargetKind::Codex => {
            crate::adapters::codex::surgical_revert(
                &binding,
                settings.codex_home_override.as_deref(),
            )?;
            if let Some(env_key) = binding.managed_env_keys.first() {
                let _ = crate::env_inject::remove_codex_env(&settings, env_key);
            }
        }
        TargetKind::Pi => {
            crate::adapters::pi::surgical_revert(
                &binding,
                settings.pi_agent_dir_override.as_deref(),
            )?;
        }
        TargetKind::Prime => {
            crate::adapters::prime::surgical_revert(
                &binding,
                settings.prime_agent_dir_override.as_deref(),
            )?;
        }
    }
    state
        .db
        .with_conn(|c| repo::binding::delete_binding(c, target))?;
    crate::tray::request_tray_menu_sync(&app);
    Ok(())
}

/// 见 `apply_site`：恢复官方配置同样会写盘 + 备份 + 剪枝。
#[tauri::command(async)]
pub fn restore_official_target(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    target: TargetKind,
) -> AppResult<()> {
    let _lock = try_lock_target(target.as_str())?;
    let settings = state.db.with_conn(repo::settings::get_settings)?;
    let binding = state
        .db
        .with_conn(|c| repo::binding::get_binding(c, target))?;

    let applied_at = Utc::now().timestamp_millis();
    let backup_root = backups_dir()?
        .join(target.as_str())
        .join(format!("{}", applied_at));
    fs::create_dir_all(&backup_root)?;

    let (site_name, model_id) = match &binding {
        Some(b) => (b.site_name_snapshot.as_str(), b.model_id.as_str()),
        None => ("official", "official"),
    };

    let mut targets_to_prune = std::collections::HashSet::new();

    let result = match target {
        TargetKind::ClaudeCode => crate::adapters::claude_code::restore_official(
            settings.claude_home_override.as_deref(),
            &backup_root,
        )
        .map(|_| ()),
        TargetKind::Codex => crate::adapters::codex::restore_official(
            binding.as_ref(),
            settings.codex_home_override.as_deref(),
            &backup_root,
        )
        .map(|outcome| {
            for key in &outcome.env_keys {
                let _ = crate::env_inject::remove_codex_env(&settings, key);
            }
        }),
        TargetKind::Pi => crate::adapters::pi::restore_official(
            binding.as_ref(),
            settings.pi_agent_dir_override.as_deref(),
            &backup_root,
        )
        .map(|_| ()),
        TargetKind::Prime => crate::adapters::prime::restore_official(
            binding.as_ref(),
            settings.prime_agent_dir_override.as_deref(),
            &backup_root,
        )
        .map(|_| ()),
    };

    match result {
        Ok(()) => {
            if binding.is_some() {
                state
                    .db
                    .with_conn(|c| repo::binding::delete_binding(c, target))?;
            }
            finalize_backup_dir(
                &backup_root,
                target,
                site_name,
                model_id,
                None,
                applied_at,
                settings.max_backup_copies,
                &mut targets_to_prune,
            );
            // revert_target 只处理一个目标，立即剪枝
            for t in targets_to_prune {
                let _ = backup::prune_target_backups(t, settings.max_backup_copies);
            }
            crate::tray::request_tray_menu_sync(&app);
            Ok(())
        }
        Err(e) => {
            finalize_backup_dir(
                &backup_root,
                target,
                site_name,
                model_id,
                None,
                applied_at,
                settings.max_backup_copies,
                &mut targets_to_prune,
            );
            // 失败时也剪枝
            for t in targets_to_prune {
                let _ = backup::prune_target_backups(t, settings.max_backup_copies);
            }
            Err(e)
        }
    }
}

#[tauri::command]
pub fn list_apply_records(
    state: State<'_, AppState>,
    limit: Option<i64>,
) -> AppResult<Vec<ApplyRecordDto>> {
    state
        .db
        .with_conn(|c| repo::apply::list_records(c, limit.unwrap_or(50)))
}

#[tauri::command]
pub fn list_backups(
    state: State<'_, AppState>,
    target: Option<String>,
) -> AppResult<Vec<BackupInfo>> {
    let root = backups_dir()?;
    if !root.exists() {
        return Ok(vec![]);
    }
    let targets: Vec<TargetKind> = if let Some(t) = target.as_deref().and_then(TargetKind::parse) {
        vec![t]
    } else {
        vec![
            TargetKind::ClaudeCode,
            TargetKind::Codex,
            TargetKind::Pi,
            TargetKind::Prime,
        ]
    };
    backup::list_backups_in(&root, &targets, |dir| {
        state
            .db
            .with_conn(|c| repo::apply::find_record_by_backup_dir(c, dir))
            .ok()
            .flatten()
            .map(|r| (Some(r.id), Some(r.site_name_snapshot), Some(r.model_id)))
            .unwrap_or((None, None, None))
    })
}

#[tauri::command]
pub fn preview_backup(id: String) -> AppResult<BackupPreview> {
    backup::preview_backup_in(&backups_dir()?, &id)
}

#[tauri::command]
pub fn delete_backup(id: String) -> AppResult<()> {
    backup::delete_backup_in(&backups_dir()?, &id)
}

/// 见 `apply_site`：恢复备份会重写整个目标配置树并做原子替换。
#[tauri::command(async)]
pub fn restore_backup(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> AppResult<()> {
    let (target, _) = backup::parse_backup_id(&id)?;
    let _lock = try_lock_target(target.as_str())?;
    let settings = state.db.with_conn(repo::settings::get_settings)?;
    backup::restore_backup_in(&backups_dir()?, &id, &settings, None)?;
    crate::tray::request_tray_menu_sync(&app);
    Ok(())
}
