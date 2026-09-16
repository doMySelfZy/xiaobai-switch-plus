use crate::app_backup;
use crate::domain::{
    BackupOperationResult, BackupOverview, LocalBackupInfo, RemoteBackupInfo, RestoreStartupResult,
    SaveWebDavConfigInput, SyncOutcome, TestWebDavConnectionInput, WebDavConfigView,
};
use crate::error::{AppError, AppResult};
use crate::repo;
use crate::repo::webdav::StoredWebDavConfig;
use crate::state::AppState;
use crate::webdav::{WebDavClient, WebDavRuntimeConfig};
use std::sync::atomic::Ordering;
use std::time::Duration;
use tauri::{AppHandle, Manager, State};

#[tauri::command]
pub fn get_webdav_config(state: State<'_, AppState>) -> AppResult<WebDavConfigView> {
    state.db.with_conn(|conn| {
        Ok(repo::webdav::get_config(conn)?
            .map(config_view)
            .unwrap_or_default())
    })
}

#[tauri::command]
pub async fn save_webdav_config(
    app: AppHandle,
    state: State<'_, AppState>,
    input: SaveWebDavConfigInput,
) -> AppResult<WebDavConfigView> {
    let interval = crate::webdav::validate_sync_interval(input.sync_interval_minutes)?;
    let max_remote_backups = crate::webdav::validate_max_backups(input.max_remote_backups)?;
    let current = state.db.with_conn(repo::webdav::get_config)?;
    let encrypted_password =
        encrypted_password_for_save(input.password.as_deref(), current.as_ref(), &state.crypto)?;
    let stored = StoredWebDavConfig {
        base_url: input.base_url.trim().into(),
        username: input.username.trim().into(),
        password_encrypted: encrypted_password,
        remote_path: input.remote_path.trim().trim_matches('/').into(),
        accept_invalid_certs: input.accept_invalid_certs,
        auto_sync_enabled: input.auto_sync_enabled,
        sync_interval_minutes: interval,
        max_remote_backups,
    };
    let runtime = runtime_config(&stored, &state.crypto)?;
    let settings = state.db.with_conn(repo::settings::get_settings)?;
    crate::webdav::validate_and_build_urls(&runtime)?;
    let _ = WebDavClient::new(runtime, &settings)?;
    state
        .db
        .with_conn(|conn| repo::webdav::save_config(conn, &stored))?;
    let view = config_view(stored);
    // 重启调度器在后台做：它要抢 `webdav_operation`，而该锁在同步期间横跨网络传输，
    // 等它完成会把「保存设置」变成几分钟的转圈。
    restart_webdav_scheduler_in_background(app);
    // 配置完成立即做一次同步决策：新机器配置后马上拉取云端数据（或首次上传）。
    crate::sync::request_sync_poll();
    Ok(view)
}

#[tauri::command]
pub async fn test_webdav_connection(
    state: State<'_, AppState>,
    input: TestWebDavConnectionInput,
) -> AppResult<()> {
    let stored = state.db.with_conn(repo::webdav::get_config)?;
    let password = match input.password.filter(|value| !value.is_empty()) {
        Some(password) => password,
        None => {
            let encrypted = stored
                .as_ref()
                .map(|config| config.password_encrypted.as_str())
                .filter(|value| !value.is_empty())
                .ok_or_else(|| AppError::new("validation_failed", "WebDAV password is required"))?;
            state.crypto.decrypt(encrypted)?
        }
    };
    let runtime = WebDavRuntimeConfig {
        base_url: input.base_url,
        username: input.username,
        password,
        remote_path: input.remote_path,
        accept_invalid_certs: input.accept_invalid_certs,
    };
    let settings = state.db.with_conn(repo::settings::get_settings)?;
    WebDavClient::new(runtime, &settings)?
        .check_connection()
        .await
}

#[tauri::command]
pub async fn create_app_backup(
    app: AppHandle,
    state: State<'_, AppState>,
    destination: String,
) -> AppResult<BackupOperationResult> {
    match destination.as_str() {
        "local" => {
            let _guard = state.webdav_operation.lock().await;
            let settings = state.db.with_conn(repo::settings::get_settings)?;
            let created =
                app_backup::create_local_backup(&state.db, "manual", settings.max_backup_copies)?;
            Ok(BackupOperationResult {
                file_name: created.file_name,
                local_path: Some(created.path.display().to_string()),
                uploaded: false,
                warning: None,
            })
        }
        "webdav" => {
            let result = run_webdav_backup(&app, "manual").await?;
            restart_webdav_scheduler_in_background(app);
            Ok(result)
        }
        _ => Err(AppError::new(
            "validation_failed",
            "backup destination must be local or webdav",
        )),
    }
}

#[tauri::command]
pub fn get_backup_overview(state: State<'_, AppState>) -> AppResult<BackupOverview> {
    let (config, sync, last_published) = state.db.with_conn(|conn| {
        Ok((
            repo::webdav::get_config(conn)?,
            repo::webdav::get_sync_status(conn)?,
            repo::sync_meta::get_meta(conn, "last_published_revision")?,
        ))
    })?;
    let next = state.next_webdav_sync_at.load(Ordering::Relaxed);
    Ok(BackupOverview {
        latest_local_backup_at: app_backup::latest_local_backup_at()?,
        webdav_configured: config.is_some(),
        webdav_auto_sync_enabled: config
            .as_ref()
            .is_some_and(|config| config.auto_sync_enabled),
        webdav_sync: sync,
        next_scheduled_at: (next > 0).then_some(next),
        sync_revision: last_published.and_then(|value| value.parse::<u64>().ok()),
    })
}

#[tauri::command]
pub fn list_local_backups() -> AppResult<Vec<LocalBackupInfo>> {
    app_backup::list_local_backups()
}

#[tauri::command]
pub async fn delete_local_backup(state: State<'_, AppState>, file_name: String) -> AppResult<()> {
    let _guard = state.webdav_operation.lock().await;
    app_backup::delete_local_backup(&file_name)
}

#[tauri::command]
pub async fn restore_local_backup(
    app: AppHandle,
    state: State<'_, AppState>,
    file_name: String,
) -> AppResult<()> {
    let _guard = state.webdav_operation.lock().await;
    let app_dir = crate::paths::app_dir()?;
    let temp_dir = tempfile::Builder::new()
        .prefix(".local-restore-")
        .tempdir_in(&app_dir)?;
    let staged = temp_dir.path().join("backup.zip");
    app_backup::stage_local_backup(&file_name, &staged)?;
    let settings = state.db.with_conn(repo::settings::get_settings)?;
    app_backup::create_local_backup(&state.db, "pre_restore", settings.max_backup_copies)?;
    // 手动恢复不携带同步目标：不提交同步记账（未指定 expected）。
    crate::pending_restore::queue_pending_restore(&staged, &app_dir, None)?;
    drop(temp_dir);
    drop(_guard);
    relaunch_after_restore(app);
    #[allow(unreachable_code)]
    Ok(())
}

/// 远端备份列表是纯读操作：不抢 `webdav_operation`。
///
/// 该锁在同步期间会横跨网络传输，抢锁会让「打开备份面板」在同步进行时干等到同步结束
/// （服务器卡住时就是几分钟）。这里的 PROPFIND 只读远端元数据，与上传/下载并发运行
/// 不会破坏任何东西——列出来的只是当时的目录快照。
#[tauri::command]
pub async fn list_webdav_backups(state: State<'_, AppState>) -> AppResult<Vec<RemoteBackupInfo>> {
    let client = client_from_state(&state)?;
    client.list_backups().await
}

#[tauri::command]
pub async fn delete_webdav_backup(state: State<'_, AppState>, file_name: String) -> AppResult<()> {
    let _guard = state.webdav_operation.lock().await;
    client_from_state(&state)?.delete_file(&file_name).await
}

#[tauri::command]
pub async fn restore_webdav_backup(
    app: AppHandle,
    state: State<'_, AppState>,
    file_name: String,
) -> AppResult<()> {
    let _guard = state.webdav_operation.lock().await;
    let settings = state.db.with_conn(repo::settings::get_settings)?;
    app_backup::create_local_backup(&state.db, "pre_restore", settings.max_backup_copies)?;
    let app_dir = crate::paths::app_dir()?;
    let temp_dir = tempfile::Builder::new()
        .prefix(".webdav-restore-")
        .tempdir_in(&app_dir)?;
    let archive = temp_dir.path().join(&file_name);
    client_from_state(&state)?
        .download_file(&file_name, &archive)
        .await?;
    // 手动恢复不携带同步目标：不提交同步记账（未指定 expected）。
    crate::pending_restore::queue_pending_restore(&archive, &app_dir, None)?;
    drop(temp_dir);
    drop(_guard);
    relaunch_after_restore(app);
    #[allow(unreachable_code)]
    Ok(())
}

pub(crate) fn relaunch_after_restore(app: AppHandle) {
    app.state::<AppState>()
        .is_quitting
        .store(true, Ordering::Relaxed);
    match crate::pending_restore::restore_relaunch_kind(cfg!(debug_assertions)) {
        crate::pending_restore::RestoreRelaunch::ExitForDevCli => {
            tracing::warn!(
                "restore queued; exiting debug process so the next `pnpm tauri dev` can apply it with Vite"
            );
            app.exit(0);
        }
        crate::pending_restore::RestoreRelaunch::RestartInPlace => app.restart(),
    }
}

#[tauri::command]
pub async fn sync_now(app: AppHandle, state: State<'_, AppState>) -> AppResult<SyncOutcome> {
    let outcome = crate::sync::run_sync(&app, &state, "manual").await?;
    if outcome.pending_restart {
        drop(state);
        relaunch_after_restore(app);
    }
    #[allow(unreachable_code)]
    Ok(outcome)
}

#[tauri::command]
pub fn take_restore_result() -> AppResult<Option<RestoreStartupResult>> {
    crate::pending_restore::take_restore_result(&crate::paths::app_dir()?)
}

/// 后台重启 WebDAV 调度器：不阻塞调用方。
///
/// `restart_webdav_scheduler` 内部要先拿到 `webdav_operation`，而正在进行的同步会持锁
/// **横跨网络传输**。调用方（保存设置、手动备份）要的是"立刻返回 + 调度器稍后重排"，
/// 而不是等同步跑完。重启失败只记日志：配置已经存好了，调度器会在下次启动/保存时重建。
fn restart_webdav_scheduler_in_background(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        if let Err(error) = restart_webdav_scheduler(app).await {
            tracing::warn!(error = %error, "failed to restart the WebDAV scheduler");
        }
    });
}

pub async fn restart_webdav_scheduler(app: AppHandle) -> AppResult<()> {
    let state = app.state::<AppState>();
    let _operation_guard = state.webdav_operation.lock().await;
    let mut handle = state.webdav_sync_handle.lock().await;
    if let Some(existing) = handle.take() {
        existing.abort();
    }
    state.next_webdav_sync_at.store(0, Ordering::Relaxed);
    let Some(config) = state.db.with_conn(repo::webdav::get_config)? else {
        return Ok(());
    };
    if !config.auto_sync_enabled {
        return Ok(());
    }
    let status = state.db.with_conn(repo::webdav::get_sync_status)?;
    let interval_ms = i64::from(config.sync_interval_minutes) * 60_000;
    let now = chrono::Utc::now().timestamp_millis();
    let next = next_scheduled_at(status.last_attempt_at, now, interval_ms);
    state.next_webdav_sync_at.store(next, Ordering::Relaxed);
    let task_app = app.clone();
    *handle = Some(tokio::spawn(async move {
        let mut next_run = next;
        loop {
            let delay_ms = (next_run - chrono::Utc::now().timestamp_millis()).max(0) as u64;
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            // 兜底触发：交给同步引擎统一决策（变更即推、打开即拉）。
            if let Err(error) =
                crate::sync::run_sync(&task_app, &task_app.state::<AppState>(), "scheduled").await
            {
                tracing::warn!(error = %error, "scheduled WebDAV sync failed");
            }
            next_run = chrono::Utc::now().timestamp_millis() + interval_ms;
            task_app
                .state::<AppState>()
                .next_webdav_sync_at
                .store(next_run, Ordering::Relaxed);
        }
    }));
    Ok(())
}

fn next_scheduled_at(last_attempt_at: Option<i64>, now: i64, interval_ms: i64) -> i64 {
    last_attempt_at
        .map(|last| (last + interval_ms).max(now))
        .unwrap_or(now + interval_ms)
}

async fn run_webdav_backup(app: &AppHandle, reason: &str) -> AppResult<BackupOperationResult> {
    let state = app.state::<AppState>();
    let _guard = state.webdav_operation.lock().await;
    state
        .db
        .with_conn(|conn| repo::webdav::record_sync_status(conn, "running", None, false))?;

    let operation = async {
        let config = state
            .db
            .with_conn(repo::webdav::get_config)?
            .ok_or_else(|| AppError::new("webdav_not_configured", "WebDAV is not configured"))?;
        let app_dir = crate::paths::app_dir()?;
        let temp_dir = tempfile::Builder::new()
            .prefix(".webdav-backup-")
            .tempdir_in(&app_dir)?;
        // 锁内只做 `VACUUM INTO`，sha256 与 zip 打包在锁外做（见 app_backup 的分段说明）。
        let snapshot = state
            .db
            .with_conn(|conn| app_backup::snapshot_database(conn, temp_dir.path()))?;
        let created = app_backup::pack_snapshot(
            &snapshot,
            &crate::paths::master_key_path()?,
            temp_dir.path(),
            reason,
        )?;
        let client = client_from_stored(&state, &config)?;
        client
            .upload_file(&created.file_name, &created.path)
            .await?;
        let device = app_backup::parse_device_from_filename(&created.file_name);
        let warning = client
            .cleanup_device_backups(&device, config.max_remote_backups)
            .await
            .err()
            .map(|error| {
                format!("Backup uploaded, but old remote backups could not be cleaned up: {error}")
            });
        Ok::<_, AppError>(BackupOperationResult {
            file_name: created.file_name,
            local_path: None,
            uploaded: true,
            warning,
        })
    }
    .await;

    match operation {
        Ok(result) => {
            let status = if result.warning.is_some() {
                "warning"
            } else {
                "success"
            };
            state.db.with_conn(|conn| {
                repo::webdav::record_sync_status(conn, status, result.warning.as_deref(), true)
            })?;
            Ok(result)
        }
        Err(error) => {
            let message = error.to_string();
            state.db.with_conn(|conn| {
                repo::webdav::record_sync_status(conn, "failed", Some(&message), false)
            })?;
            Err(error)
        }
    }
}

fn client_from_state(state: &AppState) -> AppResult<WebDavClient> {
    let stored = state
        .db
        .with_conn(repo::webdav::get_config)?
        .ok_or_else(|| AppError::new("webdav_not_configured", "WebDAV is not configured"))?;
    client_from_stored(state, &stored)
}

fn client_from_stored(state: &AppState, stored: &StoredWebDavConfig) -> AppResult<WebDavClient> {
    let settings = state.db.with_conn(repo::settings::get_settings)?;
    WebDavClient::new(runtime_config(stored, &state.crypto)?, &settings)
}

pub(crate) fn runtime_config(
    stored: &StoredWebDavConfig,
    crypto: &crate::crypto::Crypto,
) -> AppResult<WebDavRuntimeConfig> {
    Ok(WebDavRuntimeConfig {
        base_url: stored.base_url.clone(),
        username: stored.username.clone(),
        password: crypto.decrypt(&stored.password_encrypted)?,
        remote_path: stored.remote_path.clone(),
        accept_invalid_certs: stored.accept_invalid_certs,
    })
}

fn encrypted_password_for_save(
    password: Option<&str>,
    current: Option<&StoredWebDavConfig>,
    crypto: &crate::crypto::Crypto,
) -> AppResult<String> {
    match password.filter(|value| !value.is_empty()) {
        Some(password) => crypto.encrypt(password),
        None => current
            .map(|config| config.password_encrypted.clone())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                AppError::new(
                    "validation_failed",
                    "WebDAV password is required for the first configuration",
                )
            }),
    }
}

fn config_view(stored: StoredWebDavConfig) -> WebDavConfigView {
    WebDavConfigView {
        base_url: stored.base_url,
        username: stored.username,
        remote_path: stored.remote_path,
        accept_invalid_certs: stored.accept_invalid_certs,
        has_password: !stored.password_encrypted.is_empty(),
        auto_sync_enabled: stored.auto_sync_enabled,
        sync_interval_minutes: stored.sync_interval_minutes,
        max_remote_backups: stored.max_remote_backups,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_view_never_contains_the_encrypted_password() {
        let stored = StoredWebDavConfig {
            base_url: "https://dav.example.com".into(),
            username: "alice".into(),
            password_encrypted: "ciphertext".into(),
            remote_path: "xiaobai-switch".into(),
            accept_invalid_certs: false,
            auto_sync_enabled: false,
            sync_interval_minutes: 60,
            max_remote_backups: 10,
        };
        let serialized = serde_json::to_string(&config_view(stored)).unwrap();
        assert!(!serialized.contains("ciphertext"));
        assert!(serialized.contains("hasPassword"));
    }

    #[test]
    fn scheduler_runs_overdue_work_now_and_resets_after_manual_attempt() {
        let now = 10_000;
        let interval = 3_600;
        assert_eq!(next_scheduled_at(None, now, interval), now + interval);
        assert_eq!(next_scheduled_at(Some(1_000), now, interval), now);
        assert_eq!(next_scheduled_at(Some(now), now, interval), now + interval);
    }

    #[test]
    fn password_is_encrypted_and_blank_input_preserves_the_saved_value() {
        let crypto = crate::crypto::Crypto::from_key([5_u8; 32]);
        let existing_ciphertext = crypto.encrypt("old-secret").unwrap();
        let stored = StoredWebDavConfig {
            base_url: "https://dav.example.com".into(),
            username: "alice".into(),
            password_encrypted: existing_ciphertext.clone(),
            remote_path: "xiaobai-switch".into(),
            accept_invalid_certs: false,
            auto_sync_enabled: false,
            sync_interval_minutes: 60,
            max_remote_backups: 10,
        };
        assert_eq!(
            encrypted_password_for_save(Some(""), Some(&stored), &crypto).unwrap(),
            existing_ciphertext
        );
        let replacement =
            encrypted_password_for_save(Some("new-secret"), Some(&stored), &crypto).unwrap();
        assert_ne!(replacement, "new-secret");
        assert_eq!(crypto.decrypt(&replacement).unwrap(), "new-secret");
        assert!(encrypted_password_for_save(None, None, &crypto).is_err());
    }
}
