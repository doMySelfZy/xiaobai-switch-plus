//! 代理服务生命周期与连接处理。
//!
//! 只监听 127.0.0.1，HTTP/1.1。停止靠 oneshot 信号唤醒 accept 循环；
//! 运行时句柄存 `AppState.local_proxy`。
//!
//! 请求处理实时读数据库（绑定/站点/密钥/请求头），所以"换站点、改请求头"不需要重启
//! 代理。端口与路径口令属于监听面，启动时固定，改动由命令层重启代理。

use crate::domain::{LocalProxyRequestLogEntry, LocalProxyStatus, TargetKind};
use crate::error::{AppError, AppResult};
use crate::local_proxy::forward;
use crate::local_proxy::log::RequestLog;
use crate::local_proxy::routing;
use crate::state::AppState;
use http_body_util::BodyExt;
use hyper::body::Incoming;
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::Manager;
use tokio::net::TcpListener;
use tokio::sync::oneshot;

#[derive(Default)]
pub struct ProxyStats {
    pub total: AtomicU64,
    pub success: AtomicU64,
    pub failed: AtomicU64,
    pub active_connections: AtomicU64,
}

/// 一次转发所需的站点上下文：站点、明文密钥、请求头覆盖、应用设置。
pub struct ResolvedSite {
    pub site: crate::domain::SiteRow,
    pub api_key: String,
    pub overrides: Vec<crate::domain::ProxyHeader>,
    pub settings: crate::domain::AppSettings,
}

/// 站点解析器：生产环境从数据库取，测试注入固定数据。
///
/// 抽成 trait 是为了让"真实监听端口 + 真实上游 + 流式响应"这条链路可测——
/// 请求处理不再直接依赖 `tauri::AppHandle`。
pub trait SiteResolver: Send + Sync + 'static {
    fn resolve(&self, target: TargetKind) -> AppResult<ResolvedSite>;
}

/// 生产实现：目标 → 绑定 → 站点 → 激活密钥 → 解密请求头，全部来自数据库。
pub struct DatabaseResolver {
    pub app: tauri::AppHandle,
}

impl SiteResolver for DatabaseResolver {
    fn resolve(&self, target: TargetKind) -> AppResult<ResolvedSite> {
        let state = self.app.state::<AppState>();
        state.db.with_conn(|conn| {
            let site_id = crate::repo::binding::get_binding(conn, target)?
                .filter(|b| !b.orphan)
                .and_then(|b| b.site_id)
                .ok_or_else(|| {
                    AppError::new(
                        "proxy_no_binding",
                        format!("no site bound to {}", target.as_str()),
                    )
                })?;
            let site = crate::repo::site::get_site(conn, &site_id)?;
            let key = crate::repo::site_api_key::get_active(conn, &site_id)?;
            let api_key = crate::repo::site_api_key::decrypt(&state.crypto, &key)?;
            let overrides =
                crate::repo::site::get_site_proxy_headers(conn, &state.crypto, &site_id)?;
            let settings = crate::repo::settings::get_settings(conn)?;
            Ok(ResolvedSite {
                site,
                api_key,
                overrides,
                settings,
            })
        })
    }
}

/// 共享给每个连接的运行上下文。`path_token` 与监听端口在启动时固定。
pub struct ProxyContext {
    pub resolver: Arc<dyn SiteResolver>,
    pub upstream: reqwest::Client,
    pub stats: Arc<ProxyStats>,
    pub log: Arc<RequestLog>,
    pub path_token: String,
    /// 最近一次转发错误，供状态卡展示；与运行时句柄共享同一个 Arc。
    pub last_error: Arc<parking_lot::Mutex<Option<String>>>,
}

pub struct ProxyRuntime {
    pub shutdown: oneshot::Sender<()>,
    pub join: tokio::task::JoinHandle<()>,
    pub started_at: i64,
    pub stats: Arc<ProxyStats>,
    pub log: Arc<RequestLog>,
    pub port: u16,
    pub path_token: String,
    pub last_error: Arc<parking_lot::Mutex<Option<String>>>,
}

impl ProxyRuntime {
    pub fn uptime_seconds(&self) -> u64 {
        let now = chrono::Utc::now().timestamp_millis();
        ((now - self.started_at).max(0) / 1000) as u64
    }

    pub fn status(
        &self,
        targets: Vec<crate::domain::LocalProxyTargetStatus>,
    ) -> LocalProxyStatus {
        LocalProxyStatus {
            running: true,
            address: format!("{}:{}", routing::LISTEN_HOST, self.port),
            port: self.port,
            path_token: self.path_token.clone(),
            started_at: Some(self.started_at),
            uptime_seconds: self.uptime_seconds(),
            total_requests: self.stats.total.load(Ordering::Relaxed),
            success_requests: self.stats.success.load(Ordering::Relaxed),
            failed_requests: self.stats.failed.load(Ordering::Relaxed),
            active_connections: self.stats.active_connections.load(Ordering::Relaxed),
            last_error: self.last_error.lock().clone(),
            targets,
        }
    }
}

/// 启动代理。已在运行则为无操作（幂等：UI 重复点开关不报错）。
///
/// 同时把启用开关落库：它是"运行意图"，应用重启时按它决定是否自动拉起。
/// 路径口令存在设备本地文件里（`paths::ensure_local_proxy_token`），不随同步走。
pub async fn start(app: &tauri::AppHandle) -> AppResult<()> {
    let state = app.state::<AppState>();
    if state.local_proxy.lock().await.is_some() {
        return Ok(());
    }

    let settings = state.db.with_conn(crate::repo::settings::get_settings)?;
    let port = settings.local_proxy_port;
    let token = crate::paths::ensure_local_proxy_token()?;
    if !settings.local_proxy_enabled {
        let mut next = settings.clone();
        next.local_proxy_enabled = true;
        state
            .db
            .with_conn(|c| crate::repo::settings::save_settings(c, &next))?;
    }

    let upstream = forward::build_upstream_client(&settings)?;
    let listener = TcpListener::bind((routing::LISTEN_HOST, port))
        .await
        .map_err(|e| AppError::new("proxy_bind_failed", format!("cannot bind port {port}: {e}")))?;

    let stats = Arc::new(ProxyStats::default());
    let log = Arc::new(RequestLog::new());
    let last_error = Arc::new(parking_lot::Mutex::new(None));
    let context = Arc::new(ProxyContext {
        resolver: Arc::new(DatabaseResolver { app: app.clone() }),
        upstream,
        stats: stats.clone(),
        log: log.clone(),
        path_token: token.clone(),
        last_error: last_error.clone(),
    });

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let join = tokio::spawn(async move {
        serve(listener, context, shutdown_rx).await;
    });

    let mut guard = state.local_proxy.lock().await;
    if guard.is_some() {
        // 竞态：另一个调用先完成启动。放弃本次监听，保持幂等语义。
        let _ = shutdown_tx.send(());
        return Ok(());
    }
    *guard = Some(ProxyRuntime {
        shutdown: shutdown_tx,
        join,
        started_at: chrono::Utc::now().timestamp_millis(),
        stats,
        log,
        port,
        path_token: token,
        last_error,
    });
    tracing::info!(port, "local proxy started");
    Ok(())
}

/// 停止代理。
///
/// 停服务前先把接管目标改回直连：否则客户端配置仍指向本机端口，而监听已经关闭，
/// 四个 CLI 会全部连接失败且界面上毫无提示。用户重新启动代理后需要再开一次接管。
pub async fn stop(app: &tauri::AppHandle) -> AppResult<()> {
    let state = app.state::<AppState>();
    let runtime = { state.local_proxy.lock().await.take() };
    let Some(runtime) = runtime else {
        return Ok(());
    };
    let _ = runtime.shutdown.send(());
    // 不强杀：accept 循环收到信号后自然退出，在途请求跑完。
    let _ = tokio::time::timeout(Duration::from_secs(3), runtime.join).await;
    crate::local_proxy::disengage_takeover(&state, false)?;
    tracing::info!("local proxy stopped");
    Ok(())
}

async fn serve(
    listener: TcpListener,
    context: Arc<ProxyContext>,
    mut shutdown: oneshot::Receiver<()>,
) {
    loop {
        let accepted = tokio::select! {
            _ = &mut shutdown => {
                tracing::info!("local proxy accept loop stopping");
                return;
            }
            result = listener.accept() => result,
        };
        let (stream, _peer) = match accepted {
            Ok(pair) => pair,
            Err(error) => {
                tracing::warn!(error = %error, "local proxy accept failed");
                continue;
            }
        };
        let io = TokioIo::new(stream);
        let context = context.clone();
        let stats = context.stats.clone();
        stats.active_connections.fetch_add(1, Ordering::Relaxed);
        tokio::spawn(async move {
            let service = hyper::service::service_fn(move |request| {
                let context = context.clone();
                async move { Ok::<_, std::convert::Infallible>(handle(request, context).await) }
            });
            let _ = hyper::server::conn::http1::Builder::new()
                .serve_connection(io, service)
                .await;
            stats.active_connections.fetch_sub(1, Ordering::Relaxed);
        });
    }
}

/// 一次请求的结果：响应体 + 用于统计/日志的状态与错误摘要。
struct Outcome {
    response: Response<forward::BoxBody>,
    status: u16,
    error: Option<String>,
}

impl Outcome {
    fn error(status: u16, message: &str) -> Self {
        Self {
            response: forward::error_response(status, message),
            status,
            error: Some(message.to_string()),
        }
    }
}

async fn handle(
    request: Request<Incoming>,
    context: Arc<ProxyContext>,
) -> Response<forward::BoxBody> {
    let started = Instant::now();
    let method = request.method().to_string();
    let full_path = request.uri().path().to_string();
    let query = request.uri().query().map(|q| q.to_string());

    // 口令不匹配一律 404：既不暴露"这里有个代理"，也不让本机其他程序蹭用站点密钥。
    let Some(target) = routing::parse_target_from_path(&context.path_token, &full_path) else {
        let outcome = Outcome::error(404, "not found");
        finish(&context, &method, &full_path, &outcome, started);
        return outcome.response;
    };

    let outcome = forward_request(request, target, &context, query.as_deref()).await;
    finish(&context, &method, &full_path, &outcome, started);
    outcome.response
}

fn finish(
    context: &ProxyContext,
    method: &str,
    full_path: &str,
    outcome: &Outcome,
    started: Instant,
) {
    context.stats.total.fetch_add(1, Ordering::Relaxed);
    if outcome.status < 400 {
        context.stats.success.fetch_add(1, Ordering::Relaxed);
    } else {
        context.stats.failed.fetch_add(1, Ordering::Relaxed);
        if let Some(error) = &outcome.error {
            *context.last_error.lock() = Some(error.clone());
        }
    }
    context.log.push(LocalProxyRequestLogEntry {
        id: 0,
        at: chrono::Utc::now().timestamp_millis(),
        target: routing::parse_target_from_path(&context.path_token, full_path)
            .map(|t| t.as_str().to_string())
            .unwrap_or_else(|| "-".into()),
        method: method.to_string(),
        // 含口令的原始路径绝不进日志；解析不出目标时只记根路径。
        path: routing::sanitize_log_path(&context.path_token, full_path),
        status: outcome.status,
        duration_ms: started.elapsed().as_millis() as u64,
        error: outcome.error.clone(),
    });
}

/// 单次转发：解析上游、拼请求头、流式转发、流式回写。
async fn forward_request(
    request: Request<Incoming>,
    target: TargetKind,
    context: &ProxyContext,
    query: Option<&str>,
) -> Outcome {
    // 目标 → 绑定 → 站点 → 密钥，全部来自数据库；绝不接受请求侧指定的上游。
    let resolved = match context.resolver.resolve(target) {
        Ok(value) => value,
        Err(error) => return Outcome::error(502, &error.to_string()),
    };
    let ResolvedSite {
        site,
        api_key,
        overrides,
        settings,
    } = resolved;

    let route = match routing::resolve_route(
        &site,
        target,
        &settings,
        &context.path_token,
        request.uri().path(),
        query,
    ) {
        Ok(route) => route,
        Err(error) => return Outcome::error(502, &error.to_string()),
    };

    if routing::is_self_loop(&route.upstream_url, &settings) {
        return Outcome::error(502, "upstream resolves back to the local proxy");
    }
    if let Err(error) = forward::assert_allowed_upstream(&route.upstream_url) {
        return Outcome::error(502, &error.to_string());
    }

    let method = match reqwest::Method::from_bytes(request.method().as_str().as_bytes()) {
        Ok(method) => method,
        Err(error) => return Outcome::error(400, &format!("unsupported method: {error}")),
    };
    let headers = match forward::merge_headers(request.headers(), &overrides, &site.id, &api_key) {
        Ok(headers) => headers,
        Err(message) => {
            // 站点的请求头配置不合法（例如从 WebDAV 同步进来的数据没经过保存层校验）。
            return Outcome::error(502, &format!("invalid proxy headers: {message}"));
        }
    };
    let body = forward::to_reqwest_body(request.into_body());

    let mut builder = context
        .upstream
        .request(method, &route.upstream_url)
        .body(reqwest::Body::wrap_stream(body));
    for (name, value) in &headers {
        builder = builder.header(name, value);
    }

    match builder.send().await {
        Ok(response) => {
            let status = response.status();
            let headers = response.headers().clone();
            Outcome {
                response: forward::response_from_upstream(status, &headers, response),
                status: status.as_u16(),
                error: None,
            }
        }
        Err(error) => {
            // 上游错误文本可能回显密钥，先脱敏再外露。
            let message = crate::model_probe::sanitize_error(&error.to_string(), &api_key);
            Outcome::error(502, &message)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn runtime_status_reports_uptime_and_counters() {
        let stats = Arc::new(ProxyStats::default());
        stats.total.store(7, Ordering::Relaxed);
        stats.success.store(6, Ordering::Relaxed);
        stats.failed.store(1, Ordering::Relaxed);
        let (shutdown, _rx) = oneshot::channel();
        let runtime = ProxyRuntime {
            shutdown,
            join: tokio::spawn(async {}),
            started_at: chrono::Utc::now().timestamp_millis() - 5_000,
            stats,
            log: Arc::new(RequestLog::new()),
            port: 18087,
            path_token: "a".repeat(32),
            last_error: Arc::new(parking_lot::Mutex::new(None)),
        };
        let status = runtime.status(Vec::new());
        assert!(status.running);
        assert_eq!(status.port, 18087);
        assert_eq!(status.address, "127.0.0.1:18087");
        assert_eq!(status.total_requests, 7);
        assert_eq!(status.success_requests, 6);
        assert_eq!(status.failed_requests, 1);
        assert!(status.uptime_seconds >= 5);
    }

    /// 端到端：真实监听端口 + mock 上游，请求真的走 `serve` → `handle` →
    /// `forward_request` 这条链路。覆盖 404（错口令）、502（无绑定）、
    /// 头注入、路径一致性与 SSE 逐块透传。
    mod integration {
        use super::*;
        use crate::domain::ProxyHeader;
        use std::sync::Mutex as StdMutex;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        const TOKEN: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        /// 测试替身值：只用于验证 `${API_KEY}` 替换链路，不是真实凭据。
        const TEST_KEY_VALUE: &str = "unit-test-placeholder";

        #[derive(Default)]
        struct Seen {
            path: Option<String>,
            headers: Vec<(String, String)>,
        }

        /// 固定数据的站点解析器，替代数据库。
        struct FixedResolver {
            site: crate::domain::SiteRow,
            overrides: Vec<ProxyHeader>,
            settings: crate::domain::AppSettings,
            bound: bool,
        }

        impl SiteResolver for FixedResolver {
            fn resolve(&self, _target: TargetKind) -> AppResult<ResolvedSite> {
                if !self.bound {
                    return Err(AppError::new("proxy_no_binding", "no site bound"));
                }
                Ok(ResolvedSite {
                    site: self.site.clone(),
                    api_key: TEST_KEY_VALUE.into(),
                    overrides: self.overrides.clone(),
                    settings: self.settings.clone(),
                })
            }
        }

        fn site_row(base_url: &str) -> crate::domain::SiteRow {
            crate::domain::SiteRow {
                id: "s1".into(),
                name: "Relay".into(),
                base_url: base_url.into(),
                base_urls: vec![base_url.into()],
                api_key_encrypted: "enc".into(),
                key_prefix: "pre".into(),
                protocol: crate::domain::SiteProtocol::OpenaiCompatible,
                claude_auth_key_style: crate::domain::ClaudeAuthKeyStyle::AnthropicAuthToken,
                notes: None,
                enabled: true,
                sort_order: 0,
                selected_model_id: None,
                last_model_fetch_at: None,
                last_model_fetch_latency_ms: None,
                last_model_fetch_error: None,
                created_at: 1,
                updated_at: 1,
                capabilities: Default::default(),
                keys: Default::default(),
                newapi_access_token_encrypted: None,
                newapi_user_id: None,
                proxy_headers_encrypted: None,
                proxy_header_count: 0,
            }
        }

        fn settings_for(proxy_port: u16) -> crate::domain::AppSettings {
            crate::domain::AppSettings {
                local_proxy_enabled: true,
                local_proxy_port: proxy_port,
                local_proxy_targets: vec![TargetKind::Codex],
                ..Default::default()
            }
        }

        /// 起一个 mock 上游，按 `chunks` 逐块写响应（用于模拟 SSE）。
        async fn mock_upstream(
            chunks: Vec<String>,
            delay_ms: u64,
        ) -> (String, Arc<StdMutex<Seen>>) {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let seen = Arc::new(StdMutex::new(Seen::default()));
            let seen_clone = seen.clone();
            tokio::spawn(async move {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                let mut buf = vec![0u8; 8192];
                let Ok(n) = socket.read(&mut buf).await else {
                    return;
                };
                let request = String::from_utf8_lossy(&buf[..n]).to_string();
                let head = request.split("\r\n\r\n").next().unwrap_or("").to_string();
                let mut lines = head.lines();
                let path = lines
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .map(str::to_string);
                let headers: Vec<(String, String)> = lines
                    .filter_map(|line| line.split_once(':'))
                    .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
                    .collect();
                {
                    let mut guard = seen_clone.lock().unwrap();
                    guard.path = path;
                    guard.headers = headers;
                }
                let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n";
                let _ = socket.write_all(head.as_bytes()).await;
                for chunk in chunks {
                    let frame = format!("{:x}\r\n{}\r\n", chunk.len(), chunk);
                    if socket.write_all(frame.as_bytes()).await.is_err() {
                        return;
                    }
                    let _ = socket.flush().await;
                    if delay_ms > 0 {
                        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                    }
                }
                let _ = socket.write_all(b"0\r\n\r\n").await;
            });
            (format!("http://{addr}"), seen)
        }

        /// 在随机端口起真实代理，返回 (代理地址, 统计, 日志, 关闭句柄)。
        async fn spawn_proxy(
            upstream: &str,
            overrides: Vec<ProxyHeader>,
            bound: bool,
        ) -> (String, Arc<ProxyStats>, Arc<RequestLog>, oneshot::Sender<()>) {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let settings = settings_for(addr.port());
            let stats = Arc::new(ProxyStats::default());
            let log = Arc::new(RequestLog::new());
            let context = Arc::new(ProxyContext {
                resolver: Arc::new(FixedResolver {
                    site: site_row(upstream),
                    overrides,
                    settings: settings.clone(),
                    bound,
                }),
                // 代理自身出站不走系统代理，避免测试环境里被拦截。
                upstream: reqwest::Client::builder()
                    .no_proxy()
                    .build()
                    .expect("client"),
                stats: stats.clone(),
                log: log.clone(),
                path_token: TOKEN.into(),
                last_error: Arc::new(parking_lot::Mutex::new(None)),
            });
            let (shutdown_tx, shutdown_rx) = oneshot::channel();
            tokio::spawn(async move {
                serve(listener, context, shutdown_rx).await;
            });
            (format!("http://{addr}"), stats, log, shutdown_tx)
        }

        fn client() -> reqwest::Client {
            reqwest::Client::builder().no_proxy().build().unwrap()
        }

        #[tokio::test]
        async fn wrong_token_gets_404_and_never_touches_upstream() {
            let (upstream, seen) = mock_upstream(vec!["pong".into()], 0).await;
            let (proxy, stats, log, shutdown) = spawn_proxy(&upstream, Vec::new(), true).await;

            let response = client()
                .post(format!("{proxy}/{}/t/codex/v1/responses", "b".repeat(32)))
                .body("")
                .send()
                .await
                .expect("proxy reachable");
            assert_eq!(response.status().as_u16(), 404);
            assert!(seen.lock().unwrap().path.is_none(), "上游不应收到请求");
            assert_eq!(stats.total.load(Ordering::Relaxed), 1);

            // 日志里的路径不含口令原文。
            let entries = log.list(10);
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0].status, 404);
            assert!(!entries[0].path.contains(TOKEN));
            let _ = shutdown.send(());
        }

        #[tokio::test]
        async fn unbound_target_gets_502() {
            let (upstream, _seen) = mock_upstream(vec!["pong".into()], 0).await;
            let (proxy, _stats, log, shutdown) = spawn_proxy(&upstream, Vec::new(), false).await;

            let response = client()
                .post(format!("{proxy}/{TOKEN}/t/codex/v1/responses"))
                .body("")
                .send()
                .await
                .expect("proxy reachable");
            assert_eq!(response.status().as_u16(), 502);
            assert_eq!(log.list(10)[0].status, 502);
            let _ = shutdown.send(());
        }

        #[tokio::test]
        async fn injected_headers_and_path_reach_the_upstream() {
            let (upstream, seen) = mock_upstream(vec!["pong".into()], 0).await;
            let overrides = vec![
                ProxyHeader {
                    name: "x-opencode-session".into(),
                    value: "${SESSION}".into(),
                    enabled: true,
                },
                ProxyHeader {
                    name: "X-Off".into(),
                    value: "no".into(),
                    enabled: false,
                },
            ];
            let (proxy, _stats, _log, shutdown) = spawn_proxy(&upstream, overrides, true).await;

            let response = client()
                .post(format!("{proxy}/{TOKEN}/t/codex/v1/responses"))
                .body("")
                .send()
                .await
                .expect("proxy reachable");
            assert_eq!(response.status().as_u16(), 200);
            assert_eq!(response.text().await.unwrap(), "pong");

            let guard = seen.lock().unwrap();
            assert_eq!(
                guard.path.as_deref(),
                Some("/v1/responses"),
                "代理转发路径必须与直连一致"
            );
            let session = guard
                .headers
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case("x-opencode-session"))
                .map(|(_, value)| value.clone());
            assert_eq!(
                session.as_deref(),
                Some(crate::local_proxy::headers::derive_session_id("s1").as_str()),
                "会话占位符应替换成按站点稳定的值"
            );
            assert!(
                !guard
                    .headers
                    .iter()
                    .any(|(name, _)| name.eq_ignore_ascii_case("X-Off")),
                "disabled 条目不得发送"
            );
            let _ = shutdown.send(());
        }

        /// SSE 必须逐块透传：任何整体缓冲都会让"读到首块"发生在全部写完之后。
        #[tokio::test]
        async fn streaming_chunks_are_forwarded_in_order() {
            let chunks = vec![
                "data: one\n\n".to_string(),
                "data: two\n\n".to_string(),
                "data: three\n\n".to_string(),
            ];
            let (upstream, _seen) = mock_upstream(chunks.clone(), 40).await;
            let (proxy, _stats, _log, shutdown) = spawn_proxy(&upstream, Vec::new(), true).await;

            let mut response = client()
                .post(format!("{proxy}/{TOKEN}/t/codex/v1/responses"))
                .header("accept", "text/event-stream")
                .body("")
                .send()
                .await
                .expect("proxy reachable");
            assert_eq!(response.status().as_u16(), 200);

            let mut received = String::new();
            let mut first_chunk_at = None;
            while let Some(chunk) = response.chunk().await.expect("stream readable") {
                if first_chunk_at.is_none() {
                    first_chunk_at = Some(Instant::now());
                }
                received.push_str(&String::from_utf8_lossy(&chunk));
            }
            assert!(first_chunk_at.is_some(), "应至少收到一块数据");
            for expected in &chunks {
                assert!(received.contains(expected.trim()), "缺少块: {expected:?}");
            }
            let one = received.find("data: one").expect("first chunk present");
            let three = received.find("data: three").expect("last chunk present");
            assert!(one < three, "块顺序必须保持");
            let _ = shutdown.send(());
        }
    }
}
