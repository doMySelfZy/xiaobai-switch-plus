//! 代理路由：把"客户端请求代理地址"映射回"站点真实上游"。
//!
//! 核心不变式：**走代理与直连必须打到同一个上游 URL**。
//! 实现方式是两侧都由 `url_normalize::normalize_base_url` 派生：适配器写进客户端的
//! 广告地址、代理转发时使用的地址，取的都是同一个字段（claude 目标用
//! `claude_base_url`，codex 目标用 `codex_base_url`，pi/prime 按站点协议二选一），
//! 因此客户端把请求路径拼在广告地址后面，与拼在真实地址后面得到的剩余路径一致。

use crate::domain::{AppSettings, SiteProtocol, SiteRow, TargetKind};
use crate::error::{AppError, AppResult};

/// 代理只监听回环地址；不提供局域网/公网暴露。
pub const LISTEN_HOST: &str = "127.0.0.1";

/// 路径分区标记；真实目标名跟在它后面（`/t/claude_code`）。
const TARGET_SEGMENT: &str = "t";

pub fn proxy_origin(port: u16) -> String {
    format!("http://{LISTEN_HOST}:{port}")
}

/// 广告给客户端的原始基地地址（未经 normalize）。
///
/// 形态：`http://127.0.0.1:{port}/{token}/t/{target}`。
/// 适配器会对它再跑一次 `normalize_base_url`，得到与真实上游同形状的地址。
///
/// `token` 由调用方从设备本地文件取（见 `paths::ensure_local_proxy_token`）：
/// 这里保持纯函数，便于测试不碰真实数据目录。
pub fn raw_client_base_url(
    settings: &AppSettings,
    target: TargetKind,
    token: &str,
) -> AppResult<String> {
    if !crate::domain::is_valid_local_proxy_token(token) {
        return Err(AppError::new(
            "proxy_not_configured",
            "local proxy path token missing",
        ));
    }
    Ok(format!(
        "{}/{}/{}",
        proxy_origin(settings.local_proxy_port),
        token,
        target_path_segment(target)
    ))
}

/// 接管某个目标时使用的代理地址；从设备本地文件读取口令。
pub fn takeover_base_url(settings: &AppSettings, target: TargetKind) -> AppResult<String> {
    let token = crate::paths::ensure_local_proxy_token()?;
    raw_client_base_url(settings, target, &token)
}

pub fn target_path_segment(target: TargetKind) -> String {
    format!("{TARGET_SEGMENT}/{}", target.as_str())
}

/// 该目标/协议下应使用的 base_url 字段：与适配器写入 client 配置时取的字段一致。
fn normalized_base_for(
    site: &SiteRow,
    target: TargetKind,
    raw_base_url: &str,
) -> AppResult<String> {
    let preview = crate::url_normalize::normalize_base_url(raw_base_url)?;
    let use_claude = match target {
        TargetKind::ClaudeCode => true,
        TargetKind::Codex => false,
        TargetKind::Pi | TargetKind::Prime => site.protocol == SiteProtocol::Anthropic,
    };
    Ok(if use_claude {
        preview.claude_base_url
    } else {
        preview.codex_base_url
    })
}

/// 是否接管该目标（开关开 + 目标在集合里 + 代理已配置）。
pub fn is_takeover(settings: &AppSettings, target: TargetKind) -> bool {
    settings.local_proxy_enabled && settings.local_proxy_targets.contains(&target)
}

/// 接管开启时替换 `base_url` 为代理地址；否则原样返回。
///
/// 数据库里始终保存真实上游，这里只做"发给适配器之前的临时替换"，
/// 因此 `expected_fields` / `detect_status` / `rewrite_base_url` 全部沿用原逻辑，
/// 开关切换等价于"站点换了 base URL"，由既有 `route_switch` 机制完成重写。
pub fn effective_site(site: &SiteRow, target: TargetKind, settings: &AppSettings) -> SiteRow {
    let mut out = site.clone();
    if !is_takeover(settings, target) {
        return out;
    }
    match takeover_base_url(settings, target) {
        Ok(url) => out.base_url = url,
        Err(error) => {
            tracing::warn!(
                target = target.as_str(),
                error = %error,
                "local proxy takeover skipped: path token unavailable"
            );
        }
    }
    out
}

/// 一次转发所需的全部信息。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Route {
    pub target: TargetKind,
    pub upstream_url: String,
    /// 去掉代理前缀后的路径，用于日志（不含 token）。
    pub relative_path: String,
}

impl Route {
    /// 真实上游 base（已判定协议字段），供日志/调试。
    pub fn log_path(&self) -> &str {
        &self.relative_path
    }
}

/// 从请求 path 解析目标：`/{token}/t/{target}/...`。
///
/// 口令不匹配一律返回 None（调用方回 404，不泄露"这里有个代理"）。
/// 口令由监听侧传入（启动时固定），而不是每请求读设置——改口令需要重启代理。
pub fn parse_target_from_path(token: &str, path: &str) -> Option<TargetKind> {
    if !crate::domain::is_valid_local_proxy_token(token) {
        return None;
    }
    let rest = path.strip_prefix('/')?.strip_prefix(token)?;
    let rest = rest
        .strip_prefix('/')?
        .strip_prefix(TARGET_SEGMENT)?
        .strip_prefix('/')?;
    let slug = rest.split('/').next()?;
    TargetKind::parse(slug)
}

/// 日志路径脱敏：剥掉口令段，只留目标之后的相对路径。
///
/// 口令属于本机凭据，不能进日志（日志会被复述、截图、导出）。
pub fn sanitize_log_path(token: &str, path: &str) -> String {
    let prefix = format!("/{token}");
    match path.strip_prefix(&prefix) {
        Some(rest) if !rest.is_empty() => rest.to_string(),
        _ => "/".to_string(),
    }
}

/// 构造上游 URL。`token` 是当前监听使用的路径口令。
pub fn resolve_route(
    site: &SiteRow,
    target: TargetKind,
    settings: &AppSettings,
    token: &str,
    path: &str,
    query: Option<&str>,
) -> AppResult<Route> {
    let advertised_raw = raw_client_base_url(settings, target, token)?;
    let advertised = normalized_base_for(site, target, &advertised_raw)?;
    let real = normalized_base_for(site, target, &site.base_url)?;

    let advertised_path = url::Url::parse(&advertised)
        .map_err(|e| AppError::new("internal", format!("bad advertised base url: {e}")))?
        .path()
        .trim_end_matches('/')
        .to_string();

    let (base_path, remainder) = if advertised_path.is_empty() {
        ("".to_string(), path.to_string())
    } else if let Some(rest) = path.strip_prefix(&advertised_path) {
        (advertised_path.clone(), rest.to_string())
    } else {
        return Err(AppError::new(
            "proxy_route_mismatch",
            "request path does not match the advertised base URL",
        ));
    };
    let _ = base_path;

    let relative_path = if remainder.is_empty() {
        "/".to_string()
    } else if remainder.starts_with('/') {
        remainder
    } else {
        format!("/{remainder}")
    };

    let upstream_url = match query.filter(|q| !q.is_empty()) {
        Some(q) => format!("{}{}?{}", real.trim_end_matches('/'), relative_path, q),
        None => format!("{}{}", real.trim_end_matches('/'), relative_path),
    };

    Ok(Route {
        target,
        upstream_url,
        relative_path,
    })
}

/// 上游是否指向代理自己（防止配置错误导致自环）。
pub fn is_self_loop(upstream_url: &str, settings: &AppSettings) -> bool {
    let Ok(url) = url::Url::parse(upstream_url) else {
        return false;
    };
    let host_is_local = matches!(url.host_str(), Some("127.0.0.1") | Some("localhost") | Some("::1"));
    host_is_local && url.port_or_known_default() == Some(settings.local_proxy_port)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{ClaudeAuthKeyStyle, SiteKeyState};
    use std::collections::HashMap;

    const TEST_TOKEN: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn site(base_url: &str, protocol: SiteProtocol) -> SiteRow {
        SiteRow {
            id: "s1".into(),
            name: "Relay".into(),
            base_url: base_url.into(),
            base_urls: vec![base_url.into()],
            api_key_encrypted: "enc".into(),
            key_prefix: "sk-xx".into(),
            protocol,
            claude_auth_key_style: ClaudeAuthKeyStyle::AnthropicAuthToken,
            notes: None,
            enabled: true,
            sort_order: 0,
            selected_model_id: None,
            last_model_fetch_at: None,
            last_model_fetch_latency_ms: None,
            last_model_fetch_error: None,
            created_at: 1,
            updated_at: 1,
            capabilities: HashMap::new(),
            keys: SiteKeyState::default(),
            newapi_access_token_encrypted: None,
            newapi_user_id: None,
            proxy_headers_encrypted: None,
            proxy_header_count: 0,
        }
    }

    fn settings() -> AppSettings {
        AppSettings {
            local_proxy_enabled: true,
            local_proxy_port: 18087,
            local_proxy_targets: vec![
                TargetKind::ClaudeCode,
                TargetKind::Codex,
                TargetKind::Pi,
                TargetKind::Prime,
            ],
            ..Default::default()
        }
    }

    /// 直连与走代理必须落到同一上游：对每种目标/协议组合逐条比对。
    fn assert_same_as_direct(site: &SiteRow, target: TargetKind, client_suffix: &str) {
        let settings = settings();
        let advertised = normalized_base_for(
            site,
            target,
            &raw_client_base_url(&settings, target, TEST_TOKEN).unwrap(),
        )
        .unwrap();
        let direct = normalized_base_for(site, target, &site.base_url).unwrap();
        let path = format!("{}{}", url::Url::parse(&advertised).unwrap().path(), client_suffix);
        let route = resolve_route(site, target, &settings, TEST_TOKEN, &path, None).unwrap();
        assert_eq!(
            route.upstream_url,
            format!("{direct}{client_suffix}"),
            "target={} protocol={:?} base={}",
            target.as_str(),
            site.protocol,
            site.base_url
        );
    }

    #[test]
    fn claude_target_matches_direct_for_both_base_shapes() {
        for base in ["https://relay.example.com/anthropic", "https://relay.example.com/v1"] {
            let s = site(base, SiteProtocol::Anthropic);
            assert_same_as_direct(&s, TargetKind::ClaudeCode, "/v1/messages");
        }
    }

    #[test]
    fn codex_target_matches_direct_for_both_base_shapes() {
        for base in ["https://relay.example.com", "https://relay.example.com/v1"] {
            let s = site(base, SiteProtocol::OpenaiCompatible);
            assert_same_as_direct(&s, TargetKind::Codex, "/responses");
        }
    }

    #[test]
    fn pi_and_prime_match_direct_per_protocol() {
        for protocol in [SiteProtocol::Anthropic, SiteProtocol::OpenaiCompatible] {
            let base = "https://relay.example.com";
            let s = site(base, protocol.clone());
            let suffix = if protocol == SiteProtocol::Anthropic {
                "/v1/messages"
            } else {
                "/v1/chat/completions"
            };
            assert_same_as_direct(&s, TargetKind::Pi, suffix);
            assert_same_as_direct(&s, TargetKind::Prime, suffix);
        }
    }

    #[test]
    fn token_mismatch_yields_no_target() {
        let token = "a".repeat(32);
        let path = format!("/{}/t/claude_code/v1/messages", "b".repeat(32));
        assert!(parse_target_from_path(&token, &path).is_none());
        let ok = format!("/{token}/t/claude_code/v1/messages");
        assert_eq!(
            parse_target_from_path(&token, &ok),
            Some(TargetKind::ClaudeCode)
        );
        assert!(parse_target_from_path(&token, "/").is_none());
        assert!(parse_target_from_path(&token, &format!("/{token}/t/bogus/x")).is_none());
    }

    #[test]
    fn log_paths_never_contain_the_token() {
        let token = "a".repeat(32);
        let path = format!("/{token}/t/codex/v1/responses");
        let sanitized = sanitize_log_path(&token, &path);
        assert_eq!(sanitized, "/t/codex/v1/responses");
        assert!(!sanitized.contains(&token));
        assert_eq!(sanitize_log_path(&token, "/other"), "/");
    }

    #[test]
    fn effective_site_only_replaces_for_takeover_targets() {
        let mut settings = settings();
        let s = site("https://relay.example.com", SiteProtocol::Anthropic);
        // 接管开关开着时，effective_site 只能通过设备本地口令文件取地址；
        // 口令文件落在真实数据目录，因此这里改用纯函数断言同一逻辑：
        // 地址形态由 raw_client_base_url 决定，替换与否由 is_takeover 决定。
        assert!(is_takeover(&settings, TargetKind::ClaudeCode));
        let replaced = raw_client_base_url(&settings, TargetKind::ClaudeCode, TEST_TOKEN).unwrap();
        assert!(replaced.starts_with("http://127.0.0.1:18087/"));
        assert!(replaced.ends_with("/t/claude_code"));

        settings.local_proxy_targets = vec![TargetKind::Codex];
        assert!(
            !is_takeover(&settings, TargetKind::ClaudeCode),
            "不在接管集合里的目标不应被替换"
        );
        assert!(is_takeover(&settings, TargetKind::Codex));

        settings.local_proxy_enabled = false;
        assert!(!is_takeover(&settings, TargetKind::Codex), "总开关关闭时一律不接管");
    }

    #[test]
    fn query_string_is_preserved() {
        let settings = settings();
        let s = site("https://relay.example.com", SiteProtocol::OpenaiCompatible);
        let path = format!("/{TEST_TOKEN}/t/codex/v1/responses");
        let route = resolve_route(&s, TargetKind::Codex, &settings, TEST_TOKEN, &path, Some("a=1"))
            .unwrap();
        assert_eq!(route.upstream_url, "https://relay.example.com/v1/responses?a=1");
    }

    #[test]
    fn detects_self_loop() {
        let settings = settings();
        assert!(is_self_loop("http://127.0.0.1:18087/v1/x", &settings));
        assert!(!is_self_loop("http://127.0.0.1:9999/v1/x", &settings));
        assert!(!is_self_loop("https://relay.example.com/v1/x", &settings));
    }
}
