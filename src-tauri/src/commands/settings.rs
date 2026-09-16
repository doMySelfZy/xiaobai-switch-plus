use crate::domain::AppSettings;
use crate::error::AppResult;
use crate::paths::app_paths_dto;
use crate::repo;
use crate::state::AppState;
use std::sync::atomic::Ordering;
use tauri::State;

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> AppResult<AppSettings> {
    state.db.with_conn(repo::settings::get_settings)
}

/// 保存设置后按需剪枝：只有 `max_backup_copies` 真的变化时才扫描备份目录。
///
/// 剪枝要遍历 5 个目标（claude_code / codex / pi / prime / zcode）的备份目录，
/// 而设置页以前是每击键保存一次 —— 等于每敲一个数字就做 5 次全量目录扫描。
/// `current` 与 `merged` 都经过 `repo::settings` 的 normalize（同一套 clamp），
/// 两边相等就意味着用户没有改上限。
///
/// 行为不变：改了上限（调大或调小）仍然剪枝；剪枝失败照旧只忽略、不影响保存。
pub(crate) fn prune_if_max_backups_changed(
    previous_max: u32,
    merged_max: u32,
    prune: impl FnOnce(u32) -> AppResult<usize>,
) -> AppResult<usize> {
    if previous_max == merged_max {
        return Ok(0);
    }
    prune(merged_max)
}

#[tauri::command]
pub fn save_settings(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    partial: serde_json::Value,
) -> AppResult<AppSettings> {
    let current = state.db.with_conn(repo::settings::get_settings)?;
    let merged = repo::settings::preview_merge(&current, partial)?;
    crate::autostart::apply_pending_from_app(&app, &current, &merged)?;
    state
        .db
        .with_conn(|c| repo::settings::save_settings(c, &merged))?;
    state
        .close_to_tray
        .store(merged.close_to_tray, Ordering::Relaxed);
    state
        .start_in_tray
        .store(merged.start_in_tray, Ordering::Relaxed);
    crate::tray::request_tray_menu_sync(&app);
    let _ = prune_if_max_backups_changed(
        current.max_backup_copies,
        merged.max_backup_copies,
        crate::backup::prune_all,
    );
    Ok(merged)
}

#[tauri::command]
pub fn get_app_paths() -> AppResult<crate::paths::AppPaths> {
    app_paths_dto()
}

#[tauri::command]
pub fn preview_urls(base_url: String) -> AppResult<crate::url_normalize::UrlWritePreview> {
    crate::url_normalize::normalize_base_url(&base_url)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn unchanged_backup_limit_skips_pruning() {
        let scans = Cell::new(0);
        let removed = prune_if_max_backups_changed(30, 30, |_| {
            scans.set(scans.get() + 1);
            Ok(1)
        })
        .unwrap();
        assert_eq!(scans.get(), 0, "上限没变时不应扫描备份目录");
        assert_eq!(removed, 0);
    }

    #[test]
    fn changed_backup_limit_still_prunes_with_the_new_value() {
        let seen = Cell::new(0);
        let removed = prune_if_max_backups_changed(30, 5, |max| {
            seen.set(max);
            Ok(3)
        })
        .unwrap();
        assert_eq!(seen.get(), 5, "剪枝应拿到新的上限");
        assert_eq!(removed, 3);
    }
}
