use crate::domain::{
    AddSiteApiKeyInput, CreateSiteInput, DeepLinkSiteImportInput, DeepLinkSiteImportResult,
    SiteDto, SwitchRouteResult, SwitchSiteApiKeyResult, UpdateSiteApiKeyInput, UpdateSiteInput,
};
use crate::error::AppResult;
use crate::repo;
use crate::state::AppState;
use tauri::State;

#[tauri::command]
pub fn list_sites(state: State<'_, AppState>) -> AppResult<Vec<SiteDto>> {
    let rows = state.db.with_conn(repo::site::list_sites)?;
    Ok(rows.into_iter().map(|r| r.to_dto()).collect())
}

#[tauri::command]
pub fn get_site(state: State<'_, AppState>, id: String) -> AppResult<SiteDto> {
    Ok(state
        .db
        .with_conn(|c| repo::site::get_site(c, &id))?
        .to_dto())
}

#[tauri::command]
pub fn get_site_api_key(
    state: State<'_, AppState>,
    id: String,
    api_key_id: Option<String>,
) -> AppResult<String> {
    state
        .db
        .with_conn(|c| repo::site::get_site_api_key(c, &state.crypto, &id, api_key_id.as_deref()))
}

#[tauri::command]
pub fn get_site_newapi_token(state: State<'_, AppState>, id: String) -> AppResult<String> {
    state
        .db
        .with_conn(|c| repo::site::get_site_newapi_token(c, &state.crypto, &id))
}

#[tauri::command]
pub fn create_site(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    input: CreateSiteInput,
) -> AppResult<SiteDto> {
    let row = state
        .db
        .with_conn(|c| repo::site::create_site(c, &state.crypto, input))?;
    crate::tray::request_tray_menu_sync(&app);
    Ok(row.to_dto())
}

#[tauri::command]
pub fn import_site_from_deep_link(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    input: DeepLinkSiteImportInput,
) -> AppResult<DeepLinkSiteImportResult> {
    let result = crate::deep_link::import_site_from_deep_link(&state, input)?;
    crate::tray::request_tray_menu_sync(&app);
    Ok(result)
}

/// Base URL 变化时会连带重写已应用目标的配置文件（写盘 + 备份）。
/// `(async)`：普通 `#[tauri::command]` 的同步函数在 IPC 处理线程上执行，
/// 放在那里会卡住窗口消息泵；目标级锁语义不变。
#[tauri::command(async)]
pub fn update_site(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    id: String,
    input: UpdateSiteInput,
) -> AppResult<SiteDto> {
    let before = state.db.with_conn(|c| repo::site::get_site(c, &id))?;
    let row = state
        .db
        .with_conn(|c| repo::site::update_site(c, &state.crypto, &id, input))?;
    if before.base_url != row.base_url {
        let _ = crate::route_switch::sync_applied_urls(&state, &row);
    }
    crate::tray::request_tray_menu_sync(&app);
    Ok(row.to_dto())
}

#[tauri::command]
pub fn add_site_api_key(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    site_id: String,
    input: AddSiteApiKeyInput,
) -> AppResult<SiteDto> {
    let _lock = crate::lock::try_lock_site(&site_id)?;
    let row = state.db.with_conn(|c| {
        repo::site_api_key::add(
            c,
            &state.crypto,
            &site_id,
            input.label.as_deref(),
            &input.api_key,
        )?;
        repo::site::get_site(c, &site_id)
    })?;
    crate::tray::request_tray_menu_sync(&app);
    Ok(row.to_dto())
}

#[tauri::command]
pub fn update_site_api_key(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    site_id: String,
    api_key_id: String,
    input: UpdateSiteApiKeyInput,
) -> AppResult<SiteDto> {
    let _lock = crate::lock::try_lock_site(&site_id)?;
    let row = state.db.with_conn(|c| {
        repo::site_api_key::update(
            c,
            &state.crypto,
            &site_id,
            &api_key_id,
            input.label.as_deref(),
            input.api_key.as_deref(),
        )?;
        repo::site::get_site(c, &site_id)
    })?;
    crate::tray::request_tray_menu_sync(&app);
    Ok(row.to_dto())
}

#[tauri::command]
pub fn delete_site_api_key(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    site_id: String,
    api_key_id: String,
) -> AppResult<SiteDto> {
    let _lock = crate::lock::try_lock_site(&site_id)?;
    let row = state.db.with_conn(|c| {
        repo::site_api_key::delete(c, &site_id, &api_key_id)?;
        repo::site::get_site(c, &site_id)
    })?;
    crate::tray::request_tray_menu_sync(&app);
    Ok(row.to_dto())
}

#[tauri::command]
pub async fn switch_site_api_key(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    site_id: String,
    api_key_id: String,
    sync_targets: Option<bool>,
) -> AppResult<SwitchSiteApiKeyResult> {
    let result = crate::key_switch::switch_site_api_key(
        &state,
        &site_id,
        &api_key_id,
        sync_targets.unwrap_or(false),
    )
    .await?;
    crate::tray::request_tray_menu_sync(&app);
    Ok(result)
}

/// 见 `update_site`：换线路同样要重写所有绑定该站点的目标配置。
#[tauri::command(async)]
pub fn switch_site_route(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    site_id: String,
    base_url: String,
    apply: Option<bool>,
) -> AppResult<SwitchRouteResult> {
    let result =
        crate::route_switch::switch_site_route(&state, &site_id, &base_url, apply.unwrap_or(true))?;
    crate::tray::request_tray_menu_sync(&app);
    Ok(result)
}

/// 见 `update_site`：`cleanup_targets` 打开时要逐目标做手术式回滚（写盘 + 环境变量清理）。
#[tauri::command(async)]
pub fn delete_site(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    id: String,
    cleanup_targets: Option<bool>,
) -> AppResult<()> {
    let cleanup = cleanup_targets.unwrap_or(false);
    let settings = state.db.with_conn(repo::settings::get_settings)?;

    if cleanup {
        let bindings = state
            .db
            .with_conn(|c| repo::binding::list_bindings_for_site(c, &id))?;
        for b in bindings {
            match b.target {
                crate::domain::TargetKind::ClaudeCode => {
                    crate::adapters::claude_code::surgical_revert(
                        &b,
                        settings.claude_home_override.as_deref(),
                    )?;
                }
                crate::domain::TargetKind::Codex => {
                    crate::adapters::codex::surgical_revert(
                        &b,
                        settings.codex_home_override.as_deref(),
                    )?;
                    if let Some(env_key) = b.managed_env_keys.first() {
                        crate::env_inject::remove_codex_env(&settings, env_key)?;
                    }
                }
                crate::domain::TargetKind::Pi => {
                    crate::adapters::pi::surgical_revert(
                        &b,
                        settings.pi_agent_dir_override.as_deref(),
                    )?;
                }
                crate::domain::TargetKind::Prime => {
                    crate::adapters::prime::surgical_revert(
                        &b,
                        settings.prime_agent_dir_override.as_deref(),
                    )?;
                }
            }
            state
                .db
                .with_conn(|c| repo::binding::delete_binding(c, b.target))?;
        }
    } else {
        state
            .db
            .with_conn(|c| repo::binding::orphan_bindings_for_site(c, &id))?;
    }

    state.db.with_conn(|c| repo::site::delete_site(c, &id))?;
    crate::tray::request_tray_menu_sync(&app);
    Ok(())
}

#[tauri::command]
pub fn reorder_sites(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    ids: Vec<String>,
) -> AppResult<()> {
    state.db.with_conn(|c| {
        for (i, id) in ids.iter().enumerate() {
            c.execute(
                "UPDATE sites SET sort_order = ?2 WHERE id = ?1",
                rusqlite::params![id, i as i64],
            )?;
        }
        Ok(())
    })?;
    crate::tray::request_tray_menu_sync(&app);
    Ok(())
}

#[tauri::command]
pub fn set_selected_model(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    site_id: String,
    model_id: String,
) -> AppResult<()> {
    state
        .db
        .with_conn(|c| repo::site::set_selected_model(c, &site_id, &model_id))?;
    crate::tray::request_tray_menu_sync(&app);
    Ok(())
}
