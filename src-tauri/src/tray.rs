use crate::domain::{ApplyStatus, TargetKind};
use crate::state::AppState;
use std::sync::atomic::Ordering;
#[cfg(target_os = "macos")]
use tauri::menu::IconMenuItem;
use tauri::{
    image::Image,
    menu::{Menu, MenuItem, NativeIcon, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager,
};

pub const TRAY_ID: &str = "xiaobai-switch-plus-tray";
pub const TITLE_MAX_CHARS: usize = 40;
pub const QUICK_SITE_LIMIT: usize = 6;
const APPLY_PREFIX: &str = "apply:site:";

pub struct TrayLabels {
    pub header: &'static str,
    pub quit: &'static str,
    pub apply_header: &'static str,
    pub open_apply: &'static str,
    pub open_settings: &'static str,
    pub open_data_dir: &'static str,
    pub check_update: &'static str,
    pub claude: &'static str,
    pub codex: &'static str,
    pub pi: &'static str,
    pub prime: &'static str,
    pub applied: &'static str,
    pub stale: &'static str,
    pub orphan: &'static str,
    pub not_applied: &'static str,
    pub failed: &'static str,
}

pub fn tray_labels(language: &str) -> TrayLabels {
    let lang = language.to_ascii_lowercase();
    if lang == "en" || lang.starts_with("en-") {
        TrayLabels {
            header: "XiaoBaiSwitch Plus",
            quit: "Quit XiaoBaiSwitch Plus",
            apply_header: "Apply to…",
            open_apply: "Open Apply Center",
            open_settings: "Open Settings",
            open_data_dir: "Open Data Folder",
            check_update: "Check for Updates",
            claude: "Claude Code",
            codex: "Codex",
            pi: "Pi",
            prime: "Prime",
            applied: "Applied",
            stale: "Stale",
            orphan: "Orphan",
            not_applied: "Not applied",
            failed: "Failed",
        }
    } else {
        TrayLabels {
            header: "XiaoBaiSwitch Plus",
            quit: "退出 XiaoBaiSwitch Plus",
            apply_header: "应用到…",
            open_apply: "打开应用中心",
            open_settings: "打开设置",
            open_data_dir: "打开数据目录",
            check_update: "检查更新",
            claude: "Claude Code",
            codex: "Codex",
            pi: "Pi",
            prime: "Prime",
            applied: "已应用",
            stale: "已过期",
            orphan: "配置游离",
            not_applied: "未应用",
            failed: "失败",
        }
    }
}

pub fn truncate_label(s: &str, max_chars: usize) -> String {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let count = trimmed.chars().count();
    if count <= max_chars {
        return trimmed.to_string();
    }
    let truncated: String = trimmed.chars().take(max_chars.saturating_sub(1)).collect();
    format!("{truncated}…")
}

fn kind_label(labels: &TrayLabels, kind: TargetKind) -> &'static str {
    match kind {
        TargetKind::ClaudeCode => labels.claude,
        TargetKind::Codex => labels.codex,
        TargetKind::Pi => labels.pi,
        TargetKind::Prime => labels.prime,
    }
}

fn status_label(labels: &TrayLabels, status: ApplyStatus) -> &'static str {
    match status {
        ApplyStatus::Applied => labels.applied,
        ApplyStatus::Stale => labels.stale,
        ApplyStatus::Orphan => labels.orphan,
        ApplyStatus::NotApplied => labels.not_applied,
        ApplyStatus::Failed => labels.failed,
    }
}

pub fn format_target_status_line(
    labels: &TrayLabels,
    kind: TargetKind,
    status: ApplyStatus,
    site_name: Option<&str>,
    model_id: Option<&str>,
) -> String {
    let mut parts = vec![
        kind_label(labels, kind).to_string(),
        status_label(labels, status).to_string(),
    ];
    if !matches!(status, ApplyStatus::NotApplied) {
        if let Some(name) = site_name.map(str::trim).filter(|s| !s.is_empty()) {
            parts.push(truncate_label(name, TITLE_MAX_CHARS));
        }
        if let Some(model) = model_id.map(str::trim).filter(|s| !s.is_empty()) {
            parts.push(truncate_label(model, TITLE_MAX_CHARS));
        }
    }
    parts.join(" · ")
}

pub fn format_model_line(model_id: Option<&str>) -> Option<String> {
    let model = model_id.map(str::trim).filter(|s| !s.is_empty())?;
    Some(truncate_label(model, TITLE_MAX_CHARS))
}

pub fn format_tooltip(
    labels: &TrayLabels,
    claude: &str,
    codex: &str,
    pi: &str,
    prime: &str,
) -> String {
    format!(
        "{}\n{}\n{}\n{}\n{}",
        labels.header, claude, codex, pi, prime
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuickSite {
    pub id: String,
    pub name: String,
    pub enabled: bool,
}

/// `sites` is `(id, name, enabled_flag, selected_model)` in sort order.
pub fn pick_quick_sites(
    sites: &[(String, String, bool, Option<String>)],
    applied_site_ids: &[String],
    limit: usize,
) -> Vec<QuickSite> {
    sites
        .iter()
        .filter(|(_, _, enabled, _)| *enabled)
        .take(limit)
        .map(|(id, name, _, selected)| {
            let has_model = selected
                .as_deref()
                .map(str::trim)
                .is_some_and(|s| !s.is_empty());
            let applied = applied_site_ids.iter().any(|sid| sid == id);
            QuickSite {
                id: id.clone(),
                name: truncate_label(name, TITLE_MAX_CHARS),
                enabled: has_model || applied,
            }
        })
        .collect()
}

pub struct TraySnapshot {
    pub language: String,
    pub claude_line: String,
    pub claude_model: Option<String>,
    pub codex_line: String,
    pub codex_model: Option<String>,
    pub pi_line: String,
    pub pi_model: Option<String>,
    pub prime_line: String,
    pub prime_model: Option<String>,
    pub tooltip: String,
    pub sites: Vec<QuickSite>,
}

impl TraySnapshot {
    pub fn placeholder(language: &str) -> Self {
        let labels = tray_labels(language);
        let claude = format_target_status_line(
            &labels,
            TargetKind::ClaudeCode,
            ApplyStatus::NotApplied,
            None,
            None,
        );
        let codex = format_target_status_line(
            &labels,
            TargetKind::Codex,
            ApplyStatus::NotApplied,
            None,
            None,
        );
        let pi =
            format_target_status_line(&labels, TargetKind::Pi, ApplyStatus::NotApplied, None, None);
        let prime = format_target_status_line(
            &labels,
            TargetKind::Prime,
            ApplyStatus::NotApplied,
            None,
            None,
        );
        Self {
            language: language.to_string(),
            claude_line: claude.clone(),
            claude_model: None,
            codex_line: codex.clone(),
            codex_model: None,
            pi_line: pi.clone(),
            pi_model: None,
            prime_line: prime.clone(),
            prime_model: None,
            tooltip: format_tooltip(&labels, &claude, &codex, &pi, &prime),
            sites: vec![],
        }
    }
}

fn append_plain_item(
    app: &AppHandle,
    menu: &Menu<tauri::Wry>,
    id: &str,
    text: &str,
    enabled: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let item = MenuItem::with_id(app, id, text, enabled, None::<&str>)?;
    menu.append(&item)?;
    Ok(())
}

fn append_native_icon_item(
    app: &AppHandle,
    menu: &Menu<tauri::Wry>,
    id: &str,
    text: &str,
    enabled: bool,
    icon: NativeIcon,
) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(target_os = "macos")]
    {
        let item = IconMenuItem::with_id_and_native_icon(
            app,
            id,
            text,
            enabled,
            Some(icon),
            None::<&str>,
        )?;
        menu.append(&item)?;
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = icon;
        append_plain_item(app, menu, id, text, enabled)?;
    }
    Ok(())
}

fn load_tray_icon(app: &AppHandle) -> Image<'static> {
    if let Some(icon) = app.default_window_icon() {
        return icon.clone().to_owned();
    }
    Image::from_bytes(include_bytes!("../icons/icon.png")).unwrap_or_else(|_| {
        Image::from_bytes(include_bytes!("../icons/128x128.png")).expect("fallback tray icon")
    })
}

fn build_menu(
    app: &AppHandle,
    snapshot: &TraySnapshot,
) -> Result<Menu<tauri::Wry>, Box<dyn std::error::Error>> {
    let labels = tray_labels(&snapshot.language);
    let menu = Menu::new(app)?;

    append_plain_item(app, &menu, "header", labels.header, false)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;

    append_plain_item(app, &menu, "status_claude", &snapshot.claude_line, false)?;
    if let Some(model) = &snapshot.claude_model {
        append_plain_item(app, &menu, "status_claude_model", model, false)?;
    }
    append_plain_item(app, &menu, "status_codex", &snapshot.codex_line, false)?;
    if let Some(model) = &snapshot.codex_model {
        append_plain_item(app, &menu, "status_codex_model", model, false)?;
    }
    append_plain_item(app, &menu, "status_pi", &snapshot.pi_line, false)?;
    if let Some(model) = &snapshot.pi_model {
        append_plain_item(app, &menu, "status_pi_model", model, false)?;
    }
    append_plain_item(app, &menu, "status_prime", &snapshot.prime_line, false)?;
    if let Some(model) = &snapshot.prime_model {
        append_plain_item(app, &menu, "status_prime_model", model, false)?;
    }

    menu.append(&PredefinedMenuItem::separator(app)?)?;
    append_native_icon_item(
        app,
        &menu,
        "apply_header",
        labels.apply_header,
        false,
        NativeIcon::ListView,
    )?;
    for site in &snapshot.sites {
        append_plain_item(
            app,
            &menu,
            &format!("{APPLY_PREFIX}{}", site.id),
            &site.name,
            site.enabled,
        )?;
    }

    menu.append(&PredefinedMenuItem::separator(app)?)?;
    append_plain_item(app, &menu, "open_apply", labels.open_apply, true)?;
    append_plain_item(app, &menu, "open_settings", labels.open_settings, true)?;
    append_plain_item(app, &menu, "open_data_dir", labels.open_data_dir, true)?;
    append_native_icon_item(
        app,
        &menu,
        "check_update",
        labels.check_update,
        true,
        NativeIcon::Refresh,
    )?;

    menu.append(&PredefinedMenuItem::separator(app)?)?;
    append_native_icon_item(
        app,
        &menu,
        "quit",
        labels.quit,
        true,
        NativeIcon::StopProgress,
    )?;

    Ok(menu)
}

fn collect_snapshot(app: &AppHandle) -> TraySnapshot {
    let state = app.state::<AppState>();
    let settings = state
        .db
        .with_conn(crate::repo::settings::get_settings)
        .unwrap_or_default();
    let language = settings.language.clone();
    let labels = tray_labels(&language);

    let tools = crate::commands::targets::detect_cli_tools_cached(false);
    let statuses = crate::commands::targets::list_target_status_with_tools(&state, &tools)
        .unwrap_or_else(|err| {
            tracing::warn!("Failed to load target status for tray: {err}");
            Vec::new()
        });

    let find = |kind: TargetKind| statuses.iter().find(|s| s.kind == kind);
    let claude = find(TargetKind::ClaudeCode);
    let codex = find(TargetKind::Codex);
    let pi = find(TargetKind::Pi);
    let prime = find(TargetKind::Prime);

    let claude_line = match claude {
        Some(s) => format_target_status_line(
            &labels,
            TargetKind::ClaudeCode,
            s.status,
            s.applied_site_name.as_deref(),
            None,
        ),
        None => format_target_status_line(
            &labels,
            TargetKind::ClaudeCode,
            ApplyStatus::NotApplied,
            None,
            None,
        ),
    };
    let claude_model = claude.and_then(|s| format_model_line(s.applied_model_id.as_deref()));
    let codex_line = match codex {
        Some(s) => format_target_status_line(
            &labels,
            TargetKind::Codex,
            s.status,
            s.applied_site_name.as_deref(),
            None,
        ),
        None => format_target_status_line(
            &labels,
            TargetKind::Codex,
            ApplyStatus::NotApplied,
            None,
            None,
        ),
    };
    let codex_model = codex.and_then(|s| format_model_line(s.applied_model_id.as_deref()));
    let pi_line = match pi {
        Some(status) => format_target_status_line(
            &labels,
            TargetKind::Pi,
            status.status,
            status.applied_site_name.as_deref(),
            None,
        ),
        None => {
            format_target_status_line(&labels, TargetKind::Pi, ApplyStatus::NotApplied, None, None)
        }
    };
    let pi_model = pi.and_then(|status| format_model_line(status.applied_model_id.as_deref()));
    let prime_line = match prime {
        Some(status) => format_target_status_line(
            &labels,
            TargetKind::Prime,
            status.status,
            status.applied_site_name.as_deref(),
            None,
        ),
        None => format_target_status_line(
            &labels,
            TargetKind::Prime,
            ApplyStatus::NotApplied,
            None,
            None,
        ),
    };
    let prime_model =
        prime.and_then(|status| format_model_line(status.applied_model_id.as_deref()));

    let tooltip_claude = match claude {
        Some(s) => format_target_status_line(
            &labels,
            TargetKind::ClaudeCode,
            s.status,
            s.applied_site_name.as_deref(),
            s.applied_model_id.as_deref(),
        ),
        None => claude_line.clone(),
    };
    let tooltip_codex = match codex {
        Some(s) => format_target_status_line(
            &labels,
            TargetKind::Codex,
            s.status,
            s.applied_site_name.as_deref(),
            s.applied_model_id.as_deref(),
        ),
        None => codex_line.clone(),
    };
    let tooltip_pi = match pi {
        Some(status) => format_target_status_line(
            &labels,
            TargetKind::Pi,
            status.status,
            status.applied_site_name.as_deref(),
            status.applied_model_id.as_deref(),
        ),
        None => pi_line.clone(),
    };
    let tooltip_prime = match prime {
        Some(status) => format_target_status_line(
            &labels,
            TargetKind::Prime,
            status.status,
            status.applied_site_name.as_deref(),
            status.applied_model_id.as_deref(),
        ),
        None => prime_line.clone(),
    };

    let sites = state
        .db
        .with_conn(crate::repo::site::list_sites)
        .unwrap_or_default();
    let rows: Vec<(String, String, bool, Option<String>)> = sites
        .into_iter()
        .map(|s| (s.id, s.name, s.enabled, s.selected_model_id))
        .collect();
    let applied: Vec<String> = statuses
        .iter()
        .filter_map(|s| s.applied_site_id.clone())
        .collect();

    TraySnapshot {
        language,
        claude_line,
        claude_model,
        codex_line,
        codex_model,
        pi_line,
        pi_model,
        prime_line,
        prime_model,
        tooltip: format_tooltip(
            &labels,
            &tooltip_claude,
            &tooltip_codex,
            &tooltip_pi,
            &tooltip_prime,
        ),
        sites: pick_quick_sites(&rows, &applied, QUICK_SITE_LIMIT),
    }
}

fn apply_snapshot_to_tray(app: &AppHandle, snapshot: &TraySnapshot) {
    match build_menu(app, snapshot) {
        Ok(menu) => {
            if let Some(tray) = app.tray_by_id(TRAY_ID) {
                if let Err(err) = tray.set_tooltip(Some(&snapshot.tooltip)) {
                    tracing::warn!("Failed to set tray tooltip: {err}");
                }
                if let Err(err) = tray.set_menu(Some(menu)) {
                    tracing::warn!("Failed to set tray menu: {err}");
                }
            } else if let Err(err) = create_tray(app, &snapshot.language) {
                tracing::warn!("Failed to recreate tray: {err}");
                app.state::<AppState>()
                    .close_to_tray
                    .store(false, Ordering::Relaxed);
            }
        }
        Err(err) => tracing::warn!("Failed to build tray menu: {err}"),
    }
}

pub async fn sync_tray_menu(app: &AppHandle) -> Result<(), String> {
    let snapshot = collect_snapshot(app);
    let app_handle = app.clone();
    app.run_on_main_thread(move || {
        apply_snapshot_to_tray(&app_handle, &snapshot);
    })
    .map_err(|e| e.to_string())
}

pub fn request_tray_menu_sync(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(err) = sync_tray_menu(&app).await {
            tracing::warn!("Failed to sync tray menu: {err}");
        }
    });
}

fn handle_menu_event(app: &AppHandle, id: &str) {
    match id {
        "open_apply" => {
            crate::window_lifecycle::restore_main_window(app);
            let _ = app.emit("tray-navigate", "apply");
        }
        "open_settings" => {
            crate::window_lifecycle::restore_main_window(app);
            let _ = app.emit("tray-navigate", "settings");
        }
        "open_data_dir" => match crate::paths::app_dir() {
            Ok(dir) => {
                if let Err(err) = tauri_plugin_opener::open_path(dir, None::<&str>) {
                    tracing::warn!("Failed to open data dir from tray: {err}");
                }
            }
            Err(err) => tracing::warn!("Failed to resolve data dir from tray: {err}"),
        },
        "check_update" => {
            crate::window_lifecycle::restore_main_window(app);
            let _ = app.emit("tray-check-update", ());
        }
        "quit" => crate::window_lifecycle::request_quit(app),
        "header"
        | "apply_header"
        | "status_claude"
        | "status_claude_model"
        | "status_codex"
        | "status_codex_model"
        | "status_pi"
        | "status_pi_model"
        | "status_prime"
        | "status_prime_model" => {}
        other if other.starts_with(APPLY_PREFIX) => {
            let site_id = other[APPLY_PREFIX.len()..].to_string();
            if site_id.is_empty() {
                return;
            }
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                crate::tray_apply::apply_site_from_tray(&app, &site_id);
            });
        }
        _ => {}
    }
}

pub fn is_tray_open_click(button: MouseButton, button_state: MouseButtonState) -> bool {
    matches!(
        (button, button_state),
        (MouseButton::Left, MouseButtonState::Up)
    )
}

fn handle_tray_icon_event(tray: &TrayIcon, event: TrayIconEvent) {
    if let TrayIconEvent::Click {
        button,
        button_state,
        ..
    } = event
    {
        if is_tray_open_click(button, button_state) {
            crate::window_lifecycle::restore_main_window(tray.app_handle());
        }
    }
}

pub fn create_tray(app: &AppHandle, language: &str) -> Result<(), Box<dyn std::error::Error>> {
    let snapshot = TraySnapshot::placeholder(language);
    let menu = build_menu(app, &snapshot)?;
    let icon = load_tray_icon(app);

    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon)
        .icon_as_template(false)
        .menu(&menu)
        .tooltip(&snapshot.tooltip)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| {
            handle_menu_event(app, event.id.as_ref());
        })
        .on_tray_icon_event(|tray, event| {
            handle_tray_icon_event(tray, event);
        })
        .build(app)?;
    request_tray_menu_sync(app);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zh_cn_and_unknown_use_chinese() {
        let zh = tray_labels("zh-CN");
        assert_eq!(zh.open_apply, "打开应用中心");
        assert_eq!(zh.check_update, "检查更新");
        assert_eq!(zh.applied, "已应用");
        assert_eq!(tray_labels("zh").quit, "退出 XiaoBaiSwitch Plus");
        assert_eq!(tray_labels("fr-FR").open_settings, "打开设置");
    }

    #[test]
    fn en_labels() {
        let en = tray_labels("en-US");
        assert!(en.quit.contains("Quit"));
        assert_eq!(en.not_applied, "Not applied");
        assert_eq!(en.check_update, "Check for Updates");
        assert_eq!(tray_labels("en").open_apply, "Open Apply Center");
    }

    #[test]
    fn left_click_up_opens_window_other_clicks_do_not() {
        assert!(is_tray_open_click(MouseButton::Left, MouseButtonState::Up));
        assert!(!is_tray_open_click(
            MouseButton::Left,
            MouseButtonState::Down
        ));
        assert!(!is_tray_open_click(
            MouseButton::Right,
            MouseButtonState::Up
        ));
        assert!(!is_tray_open_click(
            MouseButton::Right,
            MouseButtonState::Down
        ));
    }

    #[test]
    fn truncates_long_labels() {
        let long = "测".repeat(TITLE_MAX_CHARS + 5);
        let formatted = truncate_label(&long, TITLE_MAX_CHARS);
        assert_eq!(formatted.chars().count(), TITLE_MAX_CHARS);
        assert!(formatted.ends_with('…'), "got: {formatted}");
        assert_eq!(truncate_label("   ", TITLE_MAX_CHARS), "");
        assert_eq!(truncate_label("ok", TITLE_MAX_CHARS), "ok");
    }

    #[test]
    fn status_line_includes_site_when_applied() {
        let labels = tray_labels("zh-CN");
        let line = format_target_status_line(
            &labels,
            TargetKind::ClaudeCode,
            ApplyStatus::Applied,
            Some("OpenRouter"),
            Some("claude-sonnet-4"),
        );
        assert!(line.contains("OpenRouter"));
        assert!(line.contains("已应用"));
        assert!(line.contains("claude-sonnet-4"));
    }

    #[test]
    fn status_line_omits_site_when_not_applied() {
        let labels = tray_labels("en-US");
        let line = format_target_status_line(
            &labels,
            TargetKind::Codex,
            ApplyStatus::NotApplied,
            Some("OpenRouter"),
            Some("gpt"),
        );
        assert!(!line.contains("OpenRouter"));
        assert!(!line.contains("gpt"));
        assert!(line.contains("Not applied"));
    }

    #[test]
    fn tooltip_has_product_and_all_targets() {
        let labels = tray_labels("zh-CN");
        let tip = format_tooltip(
            &labels,
            "Claude Code · 已应用",
            "Codex · 未应用",
            "Pi · 已应用",
            "Prime · 未应用",
        );
        assert!(tip.starts_with("XiaoBaiSwitch Plus"));
        assert!(tip.contains("Claude Code"));
        assert!(tip.contains("Codex"));
        assert!(tip.contains("Pi"));
        assert!(tip.contains("Prime"));
        assert!(!tip.contains("ZCode"), "退役目标不得回到托盘文案：{tip}");
        assert_eq!(tip.lines().count(), 5);
    }

    #[test]
    fn pick_quick_sites_filters_and_gates() {
        let sites = vec![
            ("a".into(), "Alpha".into(), true, Some("m1".into())),
            ("b".into(), "Beta".into(), false, Some("m2".into())),
            ("c".into(), "Gamma".into(), true, None),
            ("d".into(), "Delta".into(), true, Some("m3".into())),
        ];
        let picked = pick_quick_sites(&sites, &["c".into()], 2);
        assert_eq!(picked.len(), 2);
        assert_eq!(picked[0].id, "a");
        assert!(picked[0].enabled);
        assert_eq!(picked[1].id, "c");
        assert!(
            picked[1].enabled,
            "applied site stays enabled without model"
        );
    }

    #[test]
    fn pick_quick_sites_disables_unapplied_without_model() {
        let sites = vec![("z".into(), "Zed & Co".into(), true, None)];
        let picked = pick_quick_sites(&sites, &[], 6);
        assert_eq!(picked.len(), 1);
        assert!(!picked[0].enabled);
        assert_eq!(picked[0].name, "Zed & Co");
    }
}
