//! 本地代理：把四个目标 CLI 的 Base URL 指向本机代理，代理按站点改写请求头后转发。
//!
//! 动机：部分渠道在网关侧校验客户端身份或特定请求头（如 OpenCode Go 的
//! `x-opencode-session`），直接在 CLI 里填上游地址会被拒。代理不改客户端的鉴权与
//! 协议，只补/覆盖必要的头，并把请求原样流式转发到站点真实上游。
//!
//! 边界（有意不做）：不做协议转换、不做请求体改写、不做故障转移、不暴露非回环地址。

pub mod forward;
pub mod headers;
pub mod log;
pub mod routing;
pub mod server;

use crate::domain::{LocalProxyStatus, LocalProxyTargetStatus, TargetKind};
use crate::error::AppResult;
use crate::state::AppState;
use tauri::Manager;
/// 当前代理状态（未运行时也返回目标级信息，便于 UI 展示"关着但已接管"）。
pub fn status(app: &tauri::AppHandle) -> AppResult<LocalProxyStatus> {
    let state = app.state::<AppState>();
    let settings = state.db.with_conn(crate::repo::settings::get_settings)?;
    let targets = target_statuses(app, &settings);

    let guard = state
        .local_proxy
        .try_lock()
        .map_err(|_| crate::error::AppError::new("internal", "local proxy state busy"))?;
    match guard.as_ref() {
        Some(runtime) => Ok(runtime.status(targets)),
        None => Ok(LocalProxyStatus {
            running: false,
            address: format!("{}:{}", routing::LISTEN_HOST, settings.local_proxy_port),
            port: settings.local_proxy_port,
            path_token: crate::paths::ensure_local_proxy_token().unwrap_or_default(),
            started_at: None,
            uptime_seconds: 0,
            total_requests: 0,
            success_requests: 0,
            failed_requests: 0,
            active_connections: 0,
            last_error: None,
            targets,
        }),
    }
}

/// 每个目标的接管状态 + 广告地址预览 + 当前绑定站点。
pub fn target_statuses(
    app: &tauri::AppHandle,
    settings: &crate::domain::AppSettings,
) -> Vec<LocalProxyTargetStatus> {
    let state = app.state::<AppState>();
    let bindings = state
        .db
        .with_conn(crate::repo::binding::list_bindings)
        .unwrap_or_default();

    [
        TargetKind::ClaudeCode,
        TargetKind::Codex,
        TargetKind::Pi,
        TargetKind::Prime,
    ]
    .into_iter()
    .map(|target| {
            let binding = bindings.iter().find(|b| b.target == target && !b.orphan);
            let client_base_url = routing::takeover_base_url(settings, target).ok();
            LocalProxyTargetStatus {
                target: target.as_str().to_string(),
                takeover: routing::is_takeover(settings, target),
                site_id: binding.and_then(|b| b.site_id.clone()),
                site_name: binding.map(|b| b.site_name_snapshot.clone()),
                client_base_url,
            }
        })
        .collect()
}

/// 把已接管的目标改回直连，并清空接管集合。
///
/// 两个调用场景：用户停止代理（`keep_enabled = true`，代理仍是"下次启动要拉起"的
/// 意图），以及应用退出（`keep_enabled = false`，连运行意图一起收掉，避免下次启动
/// 自动拉起一个没人用的监听）。
///
/// 失败不阻塞调用方：退出路径上卡住比留下陈旧配置更糟，问题会记日志。
pub fn disengage_takeover(state: &AppState, keep_enabled: bool) -> AppResult<()> {
    let settings = state.db.with_conn(crate::repo::settings::get_settings)?;
    let targets: Vec<TargetKind> = settings
        .local_proxy_targets
        .iter()
        .copied()
        .filter(|target| {
            state
                .db
                .with_conn(|c| crate::repo::binding::get_binding(c, *target))
                .map(|binding| binding.is_some_and(|b| !b.orphan))
                .unwrap_or(false)
        })
        .collect();

    // 先落库再改文件：万一某个目标重写失败，设置里已经没有接管意图，
    // 不会出现"配置指向直连但界面显示接管中"的错位。
    let mut next = settings.clone();
    next.local_proxy_targets.clear();
    next.local_proxy_enabled = keep_enabled;
    state
        .db
        .with_conn(|c| crate::repo::settings::save_settings(c, &next))?;

    for target in targets {
        if let Err(error) = crate::route_switch::sync_applied_target(state, target, &next) {
            tracing::warn!(
                target = target.as_str(),
                error = %error,
                "local proxy takeover restore failed for target"
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::domain::{generate_local_proxy_token, is_valid_local_proxy_token};

    #[test]
    fn generated_path_tokens_are_32_hex_and_unique() {
        let first = generate_local_proxy_token();
        assert!(is_valid_local_proxy_token(&first), "必须是 32 位十六进制");
        assert_ne!(first, generate_local_proxy_token(), "每次生成都应不同");
        assert!(!is_valid_local_proxy_token("short"));
        assert!(!is_valid_local_proxy_token(&"z".repeat(32)));
    }
}
