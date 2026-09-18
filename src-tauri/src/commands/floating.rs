use std::collections::HashMap;
use std::sync::Mutex;

use once_cell::sync::Lazy;
use tauri::{AppHandle, Manager, State};

use crate::domain::{SiteQuota, SiteQuotaSummary};
use crate::error::AppResult;
use crate::floating_window;
use crate::repo;
use crate::state::AppState;

/// 悬浮窗用的额度缓存：站点 id → 最近一次探测到的额度。
///
/// 为什么要在后端存一份：悬浮窗是**独立的 webview 窗口**，拿不到主窗口 zustand store
/// 里的额度缓存；而每次刷新都对所有站点发网络请求既慢又容易触发限流。所以后端记住上次
/// 结果，悬浮窗打开时立刻有内容显示，再按设定的间隔去刷新。
struct CachedQuota {
    quota: SiteQuota,
}

static QUOTA_CACHE: Lazy<Mutex<HashMap<String, CachedQuota>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

fn cached_quota(site_id: &str) -> Option<SiteQuota> {
    QUOTA_CACHE
        .lock()
        .ok()?
        .get(site_id)
        .map(|entry| entry.quota.clone())
}

fn store_quota(site_id: &str, quota: SiteQuota) {
    if let Ok(mut cache) = QUOTA_CACHE.lock() {
        cache.insert(site_id.to_string(), CachedQuota { quota });
    }
}

/// 汇总各站点余额。抽成函数是为了让「读缓存」与「刷新后回读」共用同一份拼装逻辑。
///
/// 只返回**启用中**的站点：禁用意味着用户暂时不用它，不该继续占着悬浮窗的位置、
/// 也不该为它发探测请求。
fn summarize(state: &AppState) -> AppResult<Vec<SiteQuotaSummary>> {
    state.db.with_conn(|conn| {
        let sites = repo::site::list_sites(conn)?;
        Ok(sites
            .into_iter()
            .filter(|site| site.enabled)
            .map(|site| SiteQuotaSummary {
                quota: cached_quota(&site.id),
                site_id: site.id,
                site_name: site.name,
                enabled: site.enabled,
                sort_order: site.sort_order,
            })
            .collect())
    })
}

/// 读各站点的余额汇总（只读缓存，不发网络请求）。
///
/// 悬浮窗打开时先调这个立刻显示内容，最新值由主窗口的统一刷新逐站调
/// `refresh_site_quota` 写进缓存。
#[tauri::command]
pub fn get_all_sites_quota(state: State<'_, AppState>) -> AppResult<Vec<SiteQuotaSummary>> {
    summarize(&state)
}

/// 探测单个站点的余额并更新缓存。
///
/// 统一刷新按站点调它，而不是批量刷新所有站点：列表行的刷新指示器要跟着「该站点的
/// 模型 + 余额」这一轮走完，批量命令只能整体等最慢的那个站点，行上的圈会先停下来。
#[tauri::command]
pub async fn refresh_site_quota(
    state: State<'_, AppState>,
    site_id: String,
) -> AppResult<SiteQuota> {
    match crate::commands::quota::probe_quota_for(&state, &site_id).await {
        Ok(quota) => {
            store_quota(&site_id, quota.clone());
            Ok(quota)
        }
        Err(error) => {
            // 失败时保留上一次的余额（显示旧数字比整片「不可用」有用），
            // 同时把失败原因记进去——界面能据此说明为什么没刷新成功。
            let mut quota = cached_quota(&site_id)
                .unwrap_or_else(crate::quota_probe::empty_key_result);
            quota.error = Some(error.to_string());
            store_quota(&site_id, quota);
            tracing::warn!(site_id = %site_id, error = %error, "site quota probe failed");
            Err(error)
        }
    }
}

/// 切换悬浮窗显示/隐藏
#[tauri::command]
pub async fn toggle_floating_window(
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<bool> {
    let enabled = state.db.with_conn(|conn| {
        let settings = repo::settings::get_settings(conn)?;
        Ok(settings.floating_window.enabled)
    })?;

    if enabled {
        if let Some(window) = app.get_webview_window(floating_window::FLOATING_WINDOW_LABEL) {
            let visible = window
                .is_visible()
                .map_err(|e| crate::error::AppError::new("window_error", e.to_string()))?;
            if visible {
                floating_window::hide_floating_window(app)?;
                Ok(false)
            } else {
                floating_window::show_floating_window(app)?;
                Ok(true)
            }
        } else {
            floating_window::create_floating_window(app)?;
            Ok(true)
        }
    } else {
        Err(crate::error::AppError::new(
            "validation_failed",
            "Floating window is disabled in settings",
        ))
    }
}

/// 显示悬浮窗
#[tauri::command]
pub async fn show_floating_window_cmd(app: AppHandle) -> AppResult<()> {
    floating_window::show_floating_window(app)?;
    Ok(())
}

/// 隐藏悬浮窗
#[tauri::command]
pub async fn hide_floating_window_cmd(app: AppHandle) -> AppResult<()> {
    floating_window::hide_floating_window(app)?;
    Ok(())
}

/// 保存悬浮窗位置
#[tauri::command]
pub async fn save_floating_window_position(
    app: AppHandle,
    state: State<'_, AppState>,
    x: i32,
    y: i32,
) -> AppResult<()> {
    state.db.with_conn(|conn| {
        let mut settings = repo::settings::get_settings(conn)?;
        settings.floating_window.position_x = Some(x);
        settings.floating_window.position_y = Some(y);
        repo::settings::save_settings(conn, &settings)?;
        Ok(())
    })
}

/// 保存收起/展开状态
#[tauri::command]
pub async fn set_floating_window_collapsed(
    state: State<'_, AppState>,
    collapsed: bool,
) -> AppResult<()> {
    state.db.with_conn(|conn| {
        let mut settings = repo::settings::get_settings(conn)?;
        settings.floating_window.collapsed = collapsed;
        repo::settings::save_settings(conn, &settings)?;
        Ok(())
    })
}

/// 启用/禁用悬浮窗
#[tauri::command]
pub async fn set_floating_window_enabled(
    app: AppHandle,
    state: State<'_, AppState>,
    enabled: bool,
) -> AppResult<()> {
    state.db.with_conn(|conn| {
        let mut settings = repo::settings::get_settings(conn)?;
        settings.floating_window.enabled = enabled;
        repo::settings::save_settings(conn, &settings)?;
        Ok(())
    })?;

    if enabled {
        floating_window::show_floating_window(app)?;
    } else {
        floating_window::close_floating_window(app)?;
    }

    Ok(())
}

/// 设置自动刷新间隔（分钟）
#[tauri::command]
pub async fn set_floating_window_refresh_interval(
    state: State<'_, AppState>,
    minutes: u32,
) -> AppResult<()> {
    state.db.with_conn(|conn| {
        let mut settings = repo::settings::get_settings(conn)?;
        settings.floating_window.auto_refresh_minutes =
            crate::domain::clamp_floating_refresh_interval(minutes);
        repo::settings::save_settings(conn, &settings)?;
        Ok(())
    })
}

/// 重置悬浮窗位置到默认（右下角）
#[tauri::command]
pub async fn reset_floating_window_position(
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<()> {
    if let Some(window) = app.get_webview_window(floating_window::FLOATING_WINDOW_LABEL) {
        if let Some(monitor) = window
            .current_monitor()
            .map_err(|e| crate::error::AppError::new("window_error", e.to_string()))?
        {
            let monitor_size = monitor.size();
            let window_size = window
                .inner_size()
                .map_err(|e| crate::error::AppError::new("window_error", e.to_string()))?;
            let x = monitor_size.width as i32 - window_size.width as i32 - 20;
            let y = monitor_size.height as i32 - window_size.height as i32 - 60;

            window
                .set_position(tauri::PhysicalPosition::new(x, y))
                .map_err(|e| crate::error::AppError::new("window_error", e.to_string()))?;

            state.db.with_conn(|conn| {
                let mut settings = repo::settings::get_settings(conn)?;
                settings.floating_window.position_x = Some(x);
                settings.floating_window.position_y = Some(y);
                repo::settings::save_settings(conn, &settings)?;
                Ok(())
            })?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 缓存的读写是悬浮窗「打开即有内容」的基础，单独覆盖一下。
    #[test]
    fn quota_cache_round_trips_and_survives_unknown_sites() {
        // 未知站点返回 None，而不是 panic 或伪造数据。
        assert!(cached_quota("no-such-site").is_none());

        let mut quota = crate::quota_probe::empty_key_result();
        quota.remaining_usd = Some(12.5);
        quota.fetched_at = 111;
        store_quota("site-a", quota);

        let cached = cached_quota("site-a").expect("cached");
        assert_eq!(cached.remaining_usd, Some(12.5));
        assert_eq!(cached.fetched_at, 111);

        // 同一站点再写会覆盖，不会留下两份。
        let mut updated = crate::quota_probe::empty_key_result();
        updated.remaining_usd = Some(9.0);
        updated.fetched_at = 222;
        store_quota("site-a", updated);

        let cached = cached_quota("site-a").unwrap();
        assert_eq!(cached.remaining_usd, Some(9.0));
        assert_eq!(cached.fetched_at, 222, "second write must replace the first");
    }
}
