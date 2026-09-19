use crate::domain::{
    AppSettings, FetchModelsResult, ProtocolDetectionResult, SiteModelDto, SiteProtocol, SiteRow,
};
use crate::error::{AppError, AppResult};
use crate::url_normalize::normalize_base_url;
use chrono::Utc;
use futures_util::stream::{FuturesUnordered, StreamExt};
use serde::Deserialize;
use std::time::{Duration, Instant};
use uuid::Uuid;

#[derive(Debug, Deserialize)]
struct OpenAiModelsResponse {
    data: Option<Vec<OpenAiModel>>,
}

#[derive(Debug, Deserialize)]
struct OpenAiModel {
    id: String,
    owned_by: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AnthropicModelsResponse {
    data: Option<Vec<AnthropicModel>>,
}

#[derive(Debug, Deserialize)]
struct AnthropicModel {
    id: String,
    display_name: Option<String>,
}

const CLAUDE_CODE_UA: &str = "claude-cli/2.0.14 (external, cli)";

/// 探测类请求的单次超时，与 `quota_probe::PROBE_TIMEOUT` 同口径。
/// 改前这里写 15s，且同一个 URL 的多种鉴权组合是串行等待（5 × 15s = 最坏 75s）。
/// 降低到 5s 让连接失败的站点更快返回，避免让用户等太久。
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// 整组并行尝试的总预算：单次超时 + 一点调度余量，只作为兜底（正常情况下
/// 请求自身的超时先生效）。
const PROBE_BUDGET: Duration = Duration::from_secs(6);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AuthStyle {
    Bearer,
    XApiKey,
}

/// 同一个 URL 上的一种鉴权 / 客户端特征组合。数组顺序即优先级：
/// 更高优先级的组合成功时，结论就是它。
#[derive(Debug, Clone, Copy)]
struct Attempt {
    style: AuthStyle,
    impersonate: bool,
    protocol: SiteProtocol,
}

/// OpenAI 协议站点的拉取链：标准 Bearer → Claude Code 伪装。
const OPENAI_ATTEMPTS: [Attempt; 2] = [
    Attempt {
        style: AuthStyle::Bearer,
        impersonate: false,
        protocol: SiteProtocol::OpenaiCompatible,
    },
    Attempt {
        style: AuthStyle::Bearer,
        impersonate: true,
        protocol: SiteProtocol::OpenaiCompatible,
    },
];

/// Anthropic 协议站点的拉取链：标准 x-api-key → Bearer + 伪装 → x-api-key + 伪装。
const ANTHROPIC_ATTEMPTS: [Attempt; 3] = [
    Attempt {
        style: AuthStyle::XApiKey,
        impersonate: false,
        protocol: SiteProtocol::Anthropic,
    },
    Attempt {
        style: AuthStyle::Bearer,
        impersonate: true,
        protocol: SiteProtocol::Anthropic,
    },
    Attempt {
        style: AuthStyle::XApiKey,
        impersonate: true,
        protocol: SiteProtocol::Anthropic,
    },
];

/// 协议检测链 = 两个协议的尝试按「先 OpenAI 后 Anthropic」拼接，顺序即优先级。
const DETECTION_ATTEMPTS: [Attempt; 5] = [
    OPENAI_ATTEMPTS[0],
    OPENAI_ATTEMPTS[1],
    ANTHROPIC_ATTEMPTS[0],
    ANTHROPIC_ATTEMPTS[1],
    ANTHROPIC_ATTEMPTS[2],
];

/// 一次尝试的失败。`connection` 区分「与鉴权头无关的连接层失败」和「服务端给了答复」：
/// 前者换鉴权方式不会有别的结果，不值得继续等（design.md D1/D2 明确要求）。
#[derive(Debug)]
struct Failure {
    code: &'static str,
    message: String,
    connection: bool,
}

impl Failure {
    /// 服务端有答复（401/403/404/其它状态码、解析失败）：换鉴权方式仍可能成功。
    fn reply(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            connection: false,
        }
    }

    fn into_error(self) -> AppError {
        AppError::new(self.code, self.message)
    }
}

impl From<AppError> for Failure {
    fn from(error: AppError) -> Self {
        let AppError::Coded { code, message, .. } = error;
        Self::reply(code, message)
    }
}

/// reqwest 的失败分两类：连不上 / 超时（`connection = true`，与请求头无关）与
/// 请求发出后中途出错（可能是这个组合特有的，换一种或许能成）。
fn transport_failure(error: reqwest::Error) -> Failure {
    if error.is_timeout() {
        return Failure {
            code: "timeout",
            message: "request timed out".into(),
            connection: true,
        };
    }
    if error.is_connect() {
        return Failure {
            code: "network",
            message: error.to_string(),
            connection: true,
        };
    }
    Failure::reply("network", error.to_string())
}

enum Slot {
    Success(Vec<SiteModelDto>),
    Failed(Failure),
}

/// 优先级判定：找到最高优先级的成功者，前提是比它优先级更高的组合都已失败
/// （还有更高的在飞就说明结论未定）。返回 None 表示还没定。
fn decided_success(slots: &[Option<Slot>]) -> Option<usize> {
    for (index, slot) in slots.iter().enumerate() {
        match slot {
            Some(Slot::Success(_)) => return Some(index),
            Some(Slot::Failed(_)) => continue,
            None => return None,
        }
    }
    None
}

/// 结论已经可以下「这个地址连不上」：更高优先级的组合全部失败且其中有连接层失败，
/// 比它优先级更低的组合还在飞。返回那条失败用于报错。
fn decided_connection_failure(slots: &[Option<Slot>]) -> Option<&Failure> {
    let mut connection: Option<&Failure> = None;
    for slot in slots {
        match slot {
            Some(Slot::Failed(failure)) => {
                if failure.connection && connection.is_none() {
                    connection = Some(failure);
                }
            }
            Some(Slot::Success(_)) => return None,
            None => return connection,
        }
    }
    connection
}

/// 拉取失败时的兜底：部分中转站（如 AgentRouter）做客户端指纹检测，
/// 只放行 Claude Code 官方客户端特征，标准请求会被 401 拒绝。
fn with_claude_code_headers(
    req: reqwest::RequestBuilder,
) -> reqwest::RequestBuilder {
    req.header("User-Agent", CLAUDE_CODE_UA)
        .header("x-app", "cli")
}

fn parse_models(text: &str) -> AppResult<Vec<SiteModelDto>> {
    if let Ok(body) = serde_json::from_str::<AnthropicModelsResponse>(text) {
        return Ok(body
            .data
            .unwrap_or_default()
            .into_iter()
            .map(|m| SiteModelDto {
                id: Uuid::new_v4().to_string(),
                site_id: String::new(),
                api_key_id: String::new(),
                model_id: m.id.clone(),
                display_name: m.display_name.unwrap_or(m.id),
                owned_by: Some("anthropic".into()),
                raw: None,
                is_manual: false,
            })
            .collect());
    }
    if let Ok(body) = serde_json::from_str::<OpenAiModelsResponse>(text) {
        return Ok(body
            .data
            .unwrap_or_default()
            .into_iter()
            .map(|m| SiteModelDto {
                id: Uuid::new_v4().to_string(),
                site_id: String::new(),
                api_key_id: String::new(),
                model_id: m.id.clone(),
                display_name: m.id,
                owned_by: m.owned_by,
                raw: None,
                is_manual: false,
            })
            .collect());
    }
    Err(AppError::new(
        "invalid_response",
        "Could not parse models response. Enter model id manually.",
    ))
}

async fn attempt_models(
    client: &reqwest::Client,
    endpoint: &str,
    api_key: &str,
    style: AuthStyle,
    impersonate: bool,
) -> Result<Vec<SiteModelDto>, Failure> {
    let mut req = client.get(endpoint);
    req = match style {
        AuthStyle::Bearer => req.bearer_auth(api_key),
        AuthStyle::XApiKey => req
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01"),
    };
    if impersonate {
        req = with_claude_code_headers(req);
    }
    let resp = req.send().await.map_err(transport_failure)?;
    let status = resp.status();
    if status.as_u16() == 401 || status.as_u16() == 403 {
        return Err(Failure::reply("unauthorized", "unauthorized"));
    }
    if status.as_u16() == 404 {
        return Err(Failure::reply("not_found", "models endpoint not found"));
    }
    if !status.is_success() {
        return Err(Failure::reply("network", format!("HTTP {}", status.as_u16())));
    }
    let text = resp.text().await.map_err(transport_failure)?;
    parse_models(&text).map_err(Failure::from)
}

/// 同时发出 `attempts` 里全部组合，按优先级取第一个成功者。
///
/// 语义与改前的串行链完全一致（`attempts` 的顺序即优先级、先成功者胜），
/// 差别只在等待方式：改前串行各等一个超时周期（最坏 5 × 15s），这里并行发出、
/// 结论一定就返回，剩下的在飞请求随 future 一起取消。
/// 连接层失败（连不上 / 超时）一旦被判定为「更高优先级的组合都栽在它上面」就立刻返回，
/// 不再等后面的组合白等。
async fn attempt_group(
    client: &reqwest::Client,
    endpoint: &str,
    api_key: &str,
    attempts: &[Attempt],
) -> AppResult<(SiteProtocol, Vec<SiteModelDto>)> {
    let mut in_flight = FuturesUnordered::new();
    for (index, attempt) in attempts.iter().enumerate() {
        let style = attempt.style;
        let impersonate = attempt.impersonate;
        in_flight.push(async move {
            (
                index,
                attempt_models(client, endpoint, api_key, style, impersonate).await,
            )
        });
    }

    let mut slots: Vec<Option<Slot>> = (0..attempts.len()).map(|_| None).collect();
    let deadline = tokio::time::Instant::now() + PROBE_BUDGET;

    loop {
        let next = match tokio::time::timeout_at(deadline, in_flight.next()).await {
            Ok(Some(result)) => result,
            // 全部返回，或总预算耗尽（后者由下面的槽位状态共同决定报错）
            Ok(None) | Err(_) => break,
        };
        let (index, outcome) = next;
        slots[index] = Some(match outcome {
            Ok(models) => Slot::Success(models),
            Err(failure) => Slot::Failed(failure),
        });
        if let Some(winner) = decided_success(&slots) {
            let Some(Slot::Success(models)) = slots[winner].take() else {
                unreachable!("decided_success only points at a resolved success slot");
            };
            return Ok((attempts[winner].protocol.clone(), models));
        }
        if decided_connection_failure(&slots).is_some() {
            break;
        }
    }

    // 走到这里说明没有成功者：报最有代表性的那条失败。优先级最高的一条就是
    // 串行链改前会报的那条（例如全部 401 → `unauthorized`，全部连不上 → `network`/`timeout`），
    // 前端按错误码分类，语义与串行时一致。
    if let Some(failure) = decided_connection_failure(&slots).or_else(|| first_failure(&slots)) {
        return Err(AppError::new(failure.code, failure.message.clone()));
    }
    Err(AppError::new("timeout", "request timed out"))
}

fn first_failure(slots: &[Option<Slot>]) -> Option<&Failure> {
    slots.iter().find_map(|slot| match slot {
        Some(Slot::Failed(failure)) => Some(failure),
        _ => None,
    })
}

pub async fn fetch_models(
    site: &SiteRow,
    api_key: &str,
    settings: &AppSettings,
) -> AppResult<FetchModelsResult> {
    let preview = normalize_base_url(&site.base_url)?;
    let client = crate::http_client::build_client(settings, PROBE_TIMEOUT)?;

    let start = Instant::now();
    let endpoint = preview.models_url.clone();
    let attempts: &[Attempt] = match site.protocol {
        SiteProtocol::OpenaiCompatible => &OPENAI_ATTEMPTS,
        SiteProtocol::Anthropic => &ANTHROPIC_ATTEMPTS,
    };
    let (_, models) = attempt_group(&client, &endpoint, api_key, attempts).await?;

    let models = models
        .into_iter()
        .map(|mut model| {
            model.site_id = site.id.clone();
            model
        })
        .collect();
    let latency_ms = start.elapsed().as_millis() as u64;
    Ok(FetchModelsResult {
        models,
        latency_ms,
        endpoint,
        fetched_at: Utc::now().timestamp_millis(),
        api_key_id: String::new(),
    })
}

/// 自动检测站点协议：先尝试 OpenAI，失败则尝试 Anthropic。
/// 返回检测到的协议 + 模型预览（最多 5 个）。
pub async fn detect_protocol(
    base_url: &str,
    api_key: &str,
    settings: &AppSettings,
) -> AppResult<ProtocolDetectionResult> {
    let preview = normalize_base_url(base_url)?;
    let client = crate::http_client::build_client(settings, PROBE_TIMEOUT)?;
    let endpoint = preview.models_url.clone();

    let (detected_protocol, models) =
        attempt_group(&client, &endpoint, api_key, &DETECTION_ATTEMPTS).await?;
    Ok(ProtocolDetectionResult {
        detected_protocol,
        model_preview: models.into_iter().take(5).collect(),
        endpoint,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{ClaudeAuthKeyStyle, SiteKeyState};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn site(protocol: SiteProtocol, base_url: &str) -> SiteRow {
        SiteRow {
            id: "s1".into(),
            name: "R".into(),
            base_url: base_url.into(),
            base_urls: vec![base_url.into()],
            api_key_encrypted: "x".into(),
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
            capabilities: Default::default(),
            keys: SiteKeyState {
                active_api_key_id: None,
                api_keys: Vec::new(),
            },
            newapi_access_token_encrypted: None,
            newapi_user_id: None,
            proxy_headers_encrypted: None,
            proxy_header_count: 0,
        }
    }

    fn none_proxy() -> AppSettings {
        let mut settings = AppSettings::default();
        settings.proxy_mode = "none".into();
        settings
    }

    struct MockReply {
        status: &'static str,
        body: String,
        delay: Duration,
    }

    fn reply(status: &'static str, body: impl Into<String>) -> MockReply {
        MockReply {
            status,
            body: body.into(),
            delay: Duration::ZERO,
        }
    }

    /// 每个连接单独 spawn：并行化之后同一个 URL 上会同时挂着多个请求，
    /// 串行 accept 循环会把它们排成一队，测不出真实时序。
    async fn spawn_server<F>(handler: F) -> String
    where
        F: Fn(&str) -> MockReply + Send + Sync + 'static,
    {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let handler = Arc::new(handler);
        tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let handler = Arc::clone(&handler);
                tokio::spawn(async move {
                    let request = read_request(&mut socket).await;
                    let reply = handler(&request);
                    if !reply.delay.is_zero() {
                        tokio::time::sleep(reply.delay).await;
                    }
                    let response = format!(
                        "HTTP/1.1 {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        reply.status,
                        reply.body.len(),
                        reply.body
                    );
                    let _ = socket.write_all(response.as_bytes()).await;
                });
            }
        });
        format!("http://{address}")
    }

    async fn read_request(socket: &mut tokio::net::TcpStream) -> String {
        let mut bytes = Vec::new();
        let mut buffer = [0_u8; 2048];
        loop {
            let read = socket.read(&mut buffer).await.unwrap_or(0);
            if read == 0 {
                break;
            }
            bytes.extend_from_slice(&buffer[..read]);
            if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }
        String::from_utf8_lossy(&bytes).to_string()
    }

    const ANTHROPIC_MODELS_BODY: &str =
        r#"{"data":[{"id":"claude-sonnet-4","display_name":"Claude Sonnet 4"}]}"#;
    const OPENAI_MODELS_BODY: &str = r#"{"data":[{"id":"gpt-4o","owned_by":"openai"}]}"#;

    /// 只放行 Claude Code 客户端特征（模拟 AgentRouter 客户端检测）。
    async fn cc_gated_server() -> String {
        spawn_server(|request| {
            if request.contains("claude-cli/") {
                reply("200 OK", ANTHROPIC_MODELS_BODY)
            } else {
                reply(
                    "401 Unauthorized",
                    r#"{"error":{"message":"unauthorized client detected"}}"#,
                )
            }
        })
        .await
    }

    /// 标准 Bearer（未伪装）延迟 250ms 才回 OpenAI 形状；其余组合立刻回 Anthropic 形状。
    /// 用来验证「先成功者胜」仍然按优先级，而不是谁先回来谁赢。
    async fn slow_openai_server() -> String {
        spawn_server(|request| {
            let impersonated = request.contains("claude-cli/");
            let bearer = request
                .to_ascii_lowercase()
                .contains("authorization: bearer");
            if bearer && !impersonated {
                MockReply {
                    status: "200 OK",
                    body: OPENAI_MODELS_BODY.into(),
                    delay: Duration::from_millis(250),
                }
            } else {
                reply("200 OK", ANTHROPIC_MODELS_BODY)
            }
        })
        .await
    }

    /// 收满 `threshold` 个并发请求才统一回 401：用来证明多种组合是同时发出的
    /// （串行实现永远凑不满，只能等到请求超时）。
    async fn barrier_server(threshold: usize) -> (String, Arc<AtomicUsize>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let seen = Arc::new(AtomicUsize::new(0));
        let (ready_tx, ready_rx) = tokio::sync::watch::channel(false);
        let counter = Arc::clone(&seen);
        tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let counter = Arc::clone(&counter);
                let ready_tx = ready_tx.clone();
                let mut ready_rx = ready_rx.clone();
                tokio::spawn(async move {
                    let _ = read_request(&mut socket).await;
                    if counter.fetch_add(1, Ordering::SeqCst) + 1 >= threshold {
                        let _ = ready_tx.send(true);
                    }
                    while !*ready_rx.borrow_and_update() {
                        if ready_rx.changed().await.is_err() {
                            break;
                        }
                    }
                    let body = r#"{"error":{"message":"unauthorized client detected"}}"#;
                    let response = format!(
                        "HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = socket.write_all(response.as_bytes()).await;
                });
            }
        });
        (format!("http://{address}"), seen)
    }

    fn connection_failure() -> Failure {
        Failure {
            code: "network",
            message: "connect refused".into(),
            connection: true,
        }
    }

    #[test]
    fn priority_rule_waits_for_higher_priority_attempts_before_deciding() {
        // #0 还在飞，#1 已经成功：还不能下结论（更高优先级也许就成了）。
        let mut slots = vec![None, Some(Slot::Success(vec![]))];
        assert_eq!(decided_success(&slots), None);

        // #0 失败后，#1 的成功才成为结论。
        slots[0] = Some(Slot::Failed(Failure::reply("unauthorized", "unauthorized")));
        assert_eq!(decided_success(&slots), Some(1));
    }

    #[test]
    fn connection_failure_ends_the_group_without_waiting_for_lower_priority_attempts() {
        // #0 连不上，#1 还在飞：结论已定 —— 不必等后面白等。
        let slots = vec![Some(Slot::Failed(connection_failure())), None];
        assert_eq!(
            decided_connection_failure(&slots).map(|failure| failure.code),
            Some("network")
        );

        // 更高优先级还在飞时不下结论。
        let slots = vec![None, Some(Slot::Failed(connection_failure()))];
        assert!(decided_connection_failure(&slots).is_none());

        // 有组合成功时不算连接失败。
        let slots = vec![Some(Slot::Success(vec![]))];
        assert!(decided_connection_failure(&slots).is_none());
    }

    #[tokio::test]
    async fn falls_back_to_claude_code_headers_when_client_detection_rejects() {
        let base = cc_gated_server().await;
        let site = site(SiteProtocol::Anthropic, &base);
        let result = fetch_models(&site, "sk-real", &none_proxy()).await.unwrap();
        assert_eq!(result.models.len(), 1);
        assert_eq!(result.models[0].model_id, "claude-sonnet-4");
        assert_eq!(result.models[0].site_id, "s1");
    }

    #[tokio::test]
    async fn openai_protocol_also_falls_back_to_claude_code_headers() {
        let base = cc_gated_server().await;
        let site = site(SiteProtocol::OpenaiCompatible, &base);
        let result = fetch_models(&site, "sk-real", &none_proxy()).await.unwrap();
        assert_eq!(result.models[0].model_id, "claude-sonnet-4");
    }

    /// 优先级不能被并行打乱：低优先级的 Anthropic 组合先回，结论仍必须是 OpenAI。
    #[tokio::test]
    async fn detect_protocol_keeps_priority_when_a_lower_priority_attempt_answers_first() {
        let base = slow_openai_server().await;
        let result = detect_protocol(&base, "sk-real", &none_proxy()).await.unwrap();
        assert_eq!(result.detected_protocol, SiteProtocol::OpenaiCompatible);
        assert_eq!(result.model_preview[0].model_id, "gpt-4o");
    }

    /// 连不上时要给出「网络」语义的错误码（前端据此提示查地址/网络，而不是换 Key）。
    #[tokio::test]
    async fn detect_protocol_reports_connection_failure_when_host_is_unreachable() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);

        let error = detect_protocol(&format!("http://{address}"), "sk-real", &none_proxy())
            .await
            .unwrap_err();
        assert!(
            matches!(error.code(), "network" | "timeout"),
            "unexpected code: {}",
            error.code()
        );
    }

    /// 5 种组合必须同时发出：服务端收满 5 个请求才会回应，串行实现只能等到超时。
    #[tokio::test]
    async fn detect_protocol_fires_every_attempt_at_once() {
        let (base, seen) = barrier_server(DETECTION_ATTEMPTS.len()).await;
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            detect_protocol(&base, "sk-real", &none_proxy()),
        )
        .await
        .expect("multiple auth styles must be in flight at the same time");

        assert_eq!(seen.load(Ordering::SeqCst), DETECTION_ATTEMPTS.len());
        assert_eq!(result.unwrap_err().code(), "unauthorized");
    }
}
