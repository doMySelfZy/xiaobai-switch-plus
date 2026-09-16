mod adapters;
mod app_backup;
mod autostart;
mod backup;
mod capabilities;
mod cli_detect;
mod commands;
mod crypto;
mod db;
mod deep_link;
mod domain;
mod env_inject;
mod error;
mod floating_window;
mod http_client;
mod key_switch;
mod lock;
mod local_proxy;
mod macos_scheme;
mod mcp_registry;
mod model_probe;
mod models_fetch;
mod paths;
mod pending_restore;
mod quota_probe;
mod redact;
mod repo;
mod route_switch;
mod state;
mod sync;
mod tray;
mod tray_apply;
mod url_normalize;
mod webdav;
mod window_lifecycle;

use state::AppState;
use std::sync::atomic::Ordering;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            window_lifecycle::restore_main_window(app);
        }))
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .on_window_event(|_window, event| {
            if matches!(event, tauri::WindowEvent::Focused(true)) {
                // 切回窗口时做一次同步决策（换机后打开即最新）。
                crate::sync::request_sync_poll();
            }
        })
        .setup(|app| {
            let state = AppState::init()
                .map_err(|e| {
                    tracing::error!("failed to init app state: {e}");
                    e
                })
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
            let language = state
                .db
                .with_conn(repo::settings::get_settings)
                .map(|s| s.language)
                .unwrap_or_else(|_| "zh-CN".into());
            let start_in_tray = state.start_in_tray.load(Ordering::Relaxed);
            app.manage(state);
            let sync_daemon_app = app.handle().clone();
            crate::sync::spawn_sync_daemon(sync_daemon_app);
            let scheduler_app = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                if let Err(error) = commands::webdav::restart_webdav_scheduler(scheduler_app).await
                {
                    tracing::warn!(error = %error, "failed to initialize WebDAV scheduler");
                }
            });
            autostart::sync_from_settings(app.handle());
            commands::apply_platform_window_chrome(app);
            // 代理开关是"运行意图"：上次退出时是开的，启动就自动拉起。
            let proxy_app = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let settings = proxy_app
                    .state::<AppState>()
                    .db
                    .with_conn(repo::settings::get_settings);
                let Ok(settings) = settings else {
                    return;
                };
                if !settings.local_proxy_enabled || settings.local_proxy_targets.is_empty() {
                    return;
                }
                if let Err(error) = crate::local_proxy::server::start(&proxy_app).await {
                    tracing::warn!(error = %error, "failed to auto-start local proxy");
                }
            });
            if let Err(e) = tray::create_tray(app.handle(), &language) {
                tracing::warn!("failed to create system tray: {e}");
                app.state::<AppState>()
                    .close_to_tray
                    .store(false, Ordering::Relaxed);
            }
            // 初始化悬浮窗（如果已启用）
            if let Err(e) = floating_window::init_floating_window(app) {
                tracing::warn!("failed to initialize floating window: {e}");
            }
            if start_in_tray {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = window_lifecycle::hide_webview_window_to_tray(&w);
                }
            } else {
                window_lifecycle::reveal_app_in_dock(app.handle());
            }
            #[cfg(any(windows, target_os = "linux"))]
            {
                use tauri_plugin_deep_link::DeepLinkExt;
                if let Err(e) = app.deep_link().register_all() {
                    tracing::warn!("failed to register deep link scheme: {e}");
                }
            }
            // macOS cannot register schemes at runtime. `tauri dev` is a raw
            // binary, so Launch Services never sees CFBundleURLTypes unless we
            // drop a helper .app into ~/Applications.
            #[cfg(all(target_os = "macos", debug_assertions))]
            {
                if let Err(e) = macos_scheme::install_dev_url_handler() {
                    tracing::warn!("failed to register macOS xiaobaiswitchplus:// handler: {e}");
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_settings,
            commands::list_mcp_servers,
            commands::get_mcp_server,
            commands::save_mcp_server,
            commands::delete_mcp_server,
            commands::apply_mcp_servers,
            commands::mcp_target_paths,
            commands::search_mcp_registry,
            commands::discover_mcp_registry,
            commands::scan_existing_mcp,
            commands::import_scanned_mcp,
            commands::check_mcp_updates,
            commands::update_mcp_server,
            commands::batch_update_mcp_servers,
            commands::check_agent_updates,
            commands::get_agent_update_status,
            commands::get_all_agent_update_statuses,
            commands::update_agent,
            commands::batch_update_agents,
            commands::clear_agent_update_status,
            commands::get_agent_rules,
            commands::save_agent_rules,
            commands::agent_rules_target_paths,
            commands::save_settings,
            commands::get_app_paths,
            commands::preview_urls,
            commands::list_skills,
            commands::get_skill,
            commands::set_skill_enabled,
            commands::install_skill,
            commands::uninstall_skill,
            commands::search_skill_marketplace,
            commands::list_sites,
            commands::get_site,
            commands::get_site_api_key,
            commands::get_site_newapi_token,
            commands::add_site_api_key,
            commands::update_site_api_key,
            commands::delete_site_api_key,
            commands::switch_site_api_key,
            commands::create_site,
            commands::import_site_from_deep_link,
            commands::update_site,
            commands::switch_site_route,
            commands::delete_site,
            commands::reorder_sites,
            commands::set_selected_model,
            commands::fetch_site_models,
            commands::probe_site_api_key,
            commands::list_site_models,
            commands::get_site_thinking_preset,
            commands::save_site_thinking_preset,
            commands::delete_site_model,
            commands::clear_site_models,
            commands::probe_site_model,
            commands::test_site_connection,
            commands::probe_site_quota,
            commands::test_newapi_access,
            commands::list_target_status,
            commands::detect_cli_tools,
            commands::cleanup_orphan_target,
            commands::apply_site,
            commands::revert_target,
            commands::restore_official_target,
            commands::list_apply_records,
            commands::list_backups,
            commands::preview_backup,
            commands::delete_backup,
            commands::restore_backup,
            commands::sync_windows_chrome,
            commands::set_always_on_top,
            commands::minimize_window,
            commands::toggle_maximize_window,
            commands::open_path,
            commands::open_url,
            commands::fetch_http_text,
            commands::fetch_http_bytes,
            commands::probe_urls,
            commands::resolve_http_proxy,
            commands::local_proxy_status,
            commands::start_local_proxy,
            commands::stop_local_proxy,
            commands::set_local_proxy_takeover,
            commands::set_local_proxy_port,
            commands::list_local_proxy_requests,
            commands::clear_local_proxy_requests,
            commands::get_site_proxy_headers,
            commands::check_app_update,
            commands::take_pending_deep_link,
            commands::deep_link_requires_polling,
            commands::restore_main_window,
            commands::force_quit,
            commands::refresh_tray_menu,
            commands::get_webdav_config,
            commands::save_webdav_config,
            commands::test_webdav_connection,
            commands::create_app_backup,
            commands::get_backup_overview,
            commands::list_local_backups,
            commands::delete_local_backup,
            commands::restore_local_backup,
            commands::list_webdav_backups,
            commands::delete_webdav_backup,
            commands::restore_webdav_backup,
            commands::sync_now,
            commands::take_restore_result,
            commands::get_all_sites_quota,
            commands::toggle_floating_window,
            commands::refresh_sites_quota,
            commands::set_floating_window_collapsed,
            commands::show_floating_window_cmd,
            commands::hide_floating_window_cmd,
            commands::save_floating_window_position,
            commands::set_floating_window_enabled,
            commands::set_floating_window_refresh_interval,
            commands::reset_floating_window_position,
        ])
        .on_window_event(|window, event| {
            if window.label() != "main" {
                return;
            }
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window_lifecycle::should_close_to_tray(window.app_handle()) {
                    let _ = window_lifecycle::hide_main_window_to_tray(window);
                    api.prevent_close();
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building XiaoBaiSwitch Plus")
        .run(|app, event| {
            if let tauri::RunEvent::ExitRequested { api, .. } = &event {
                let state = app.state::<AppState>();
                if state.close_to_tray.load(Ordering::Relaxed)
                    && !state.is_quitting.load(Ordering::Relaxed)
                    && app
                        .get_webview_window("main")
                        .map(|w| !w.is_visible().unwrap_or(true))
                        .unwrap_or(false)
                {
                    api.prevent_exit();
                    return;
                }
                // 真正退出：把接管目标改回直连并收掉运行意图，避免下次启动自动拉起
                // 一个没人用的监听，或让客户端指向一个已消失的代理。
                {
                    let state = app.state::<AppState>();
                    if let Err(error) = crate::local_proxy::disengage_takeover(&state, false) {
                        tracing::warn!(error = %error, "local proxy exit restore failed");
                    }
                    let guard = state.local_proxy.try_lock();
                    if let Ok(mut guard) = guard {
                        if let Some(runtime) = guard.take() {
                            let _ = runtime.shutdown.send(());
                        }
                    }
                }
            }
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen {
                has_visible_windows,
                ..
            } = event
            {
                if !has_visible_windows {
                    window_lifecycle::restore_main_window(app);
                }
            }
        });
}
