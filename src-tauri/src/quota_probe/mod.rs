use crate::domain::{AppSettings, QuotaProbeStatus, QuotaSource, QuotaWindow, SiteQuota, SiteRow};
use crate::error::AppResult;
use crate::model_probe::sanitize_error;
use crate::url_normalize::normalize_base_url;
use chrono::{Datelike, NaiveDate, Utc};
use serde::Serialize;
use serde_json::Value;
use std::time::{Duration, Instant};
use url::Url;

const PROBE_TIMEOUT: Duration = Duration::from_secs(8);
const MAX_BODY_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub struct BillingUrls {
    pub credit_grants: String,
    pub subscription: String,
    pub usage: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Grants {
    pub remaining: Option<f64>,
    pub used: Option<f64>,
    pub total: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Subscription {
    pub limit_usd: Option<f64>,
    pub expires_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Usage {
    pub total_usage: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TokenUsage {
    pub remaining: Option<f64>,
    pub used: Option<f64>,
    pub total: Option<f64>,
    pub unit: String,
    pub unlimited: bool,
    pub expires_at: Option<i64>,
    pub has_display: bool,
}

/// Sub2API `/v1/usage` 的钱包余额结果。钱包模式下 `used` / `total` 无可靠语义，
/// 解析时刻意留空（不能用 `balance` 冒充 `total`）。
#[derive(Debug, Clone, PartialEq)]
pub struct Sub2ApiUsage {
    pub remaining: Option<f64>,
    pub used: Option<f64>,
    pub total: Option<f64>,
    pub unit: String,
    pub unlimited: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuotaDisplayType {
    Usd,
    Cny,
    Tokens,
    Custom,
    Raw,
}

#[derive(Debug, Clone, PartialEq)]
pub struct QuotaStatus {
    pub quota_per_unit: Option<f64>,
    pub display_type: QuotaDisplayType,
    pub usd_exchange_rate: Option<f64>,
    pub custom_currency_symbol: Option<String>,
    pub custom_currency_exchange_rate: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Hit {
    Grants(Grants),
    Subscription(Subscription),
    Usage(Usage),
    Token(TokenUsage),
    Status(QuotaStatus),
    Sub2ApiUsage(Sub2ApiUsage),
    InvalidData(String),
    NotFound,
    Unauthorized,
    Unsupported,
    Error(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum RoundOutcome {
    Available(SiteQuota),
    Fallback,
    Quiet(SiteQuota),
}

pub fn empty_key_result() -> SiteQuota {
    quiet(QuotaProbeStatus::Unsupported, None, 0, None)
}

pub fn strip_trailing_slash(s: &str) -> &str {
    s.trim_end_matches('/')
}

pub fn origin_without_v1(codex_base_url: &str) -> Option<String> {
    let trimmed = strip_trailing_slash(codex_base_url);
    let lower = trimmed.to_ascii_lowercase();
    let stripped = lower.strip_suffix("/v1")?;
    let origin = strip_trailing_slash(&trimmed[..stripped.len()]);
    let ol = origin.to_ascii_lowercase();
    if ol == "http://" || ol == "https://" || origin.len() < 8 {
        return None;
    }
    if ol.starts_with("http://") || ol.starts_with("https://") {
        Some(origin.to_string())
    } else {
        None
    }
}

pub fn site_origin(codex_base_url: &str) -> Option<String> {
    let parsed = Url::parse(codex_base_url).ok()?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return None;
    }
    let origin = parsed.origin().ascii_serialization();
    (origin != "null").then_some(origin)
}

pub fn public_api_bases(codex_base_url: &str) -> Vec<String> {
    let mut bases = Vec::new();
    if let Some(path_base) = origin_without_v1(codex_base_url) {
        bases.push(path_base);
    }
    if let Some(root_base) = site_origin(codex_base_url) {
        if !bases.contains(&root_base) {
            bases.push(root_base);
        }
    }
    bases
}

pub fn usage_date_range(today: NaiveDate) -> (String, String) {
    let start = NaiveDate::from_ymd_opt(today.year(), today.month(), 1).unwrap_or(today);
    let end = today.succ_opt().unwrap_or(today);
    (
        start.format("%Y-%m-%d").to_string(),
        end.format("%Y-%m-%d").to_string(),
    )
}

pub fn billing_urls(api_root: &str, today: NaiveDate) -> BillingUrls {
    let root = strip_trailing_slash(api_root);
    let (start, end) = usage_date_range(today);
    BillingUrls {
        credit_grants: format!("{root}/dashboard/billing/credit_grants"),
        subscription: format!("{root}/dashboard/billing/subscription"),
        usage: format!("{root}/dashboard/billing/usage?start_date={start}&end_date={end}"),
    }
}

pub fn token_usage_url(origin: &str) -> String {
    format!("{}/api/usage/token", strip_trailing_slash(origin))
}

pub fn quota_status_url(origin: &str) -> String {
    format!("{}/api/status", strip_trailing_slash(origin))
}

/// Sub2API 的用量/钱包端点：`{origin}/v1/usage`。
pub fn sub2api_usage_url(origin: &str) -> String {
    format!("{}/v1/usage", strip_trailing_slash(origin))
}

/// `/zen/go` must match on path-segment boundaries, so `/zen/gopher` and
/// `/zen/go-v2` are not treated as the Go channel.
fn has_zen_go_segments(path: &str) -> bool {
    let segments: Vec<&str> = path.split('/').filter(|segment| !segment.is_empty()).collect();
    segments
        .windows(2)
        .any(|pair| pair[0] == "zen" && pair[1] == "go")
}

/// OpenCode Go 渠道识别：host 必须严格等于 `opencode.ai`，且路径含 `/zen/go` 段。
/// 只接受 `https`（官方用量端点仅支持 TLS），避免把普通站点误判到该探测链。
pub fn is_opencode_go_base(base_url: &str) -> bool {
    let Ok(parsed) = Url::parse(base_url.trim()) else {
        return false;
    };
    if parsed.scheme() != "https" {
        return false;
    }
    if parsed.host_str() != Some("opencode.ai") {
        return false;
    }
    has_zen_go_segments(parsed.path())
}

/// 固定官方用量端点：origin + `/zen/go/v1/usage`（只允许 https + opencode.ai）。
pub fn opencode_go_usage_url(base_url: &str) -> Option<String> {
    #[cfg(test)]
    if let Some(url) = OPENCODE_GO_URL_OVERRIDE.with(|cell| cell.borrow().clone()) {
        return Some(url);
    }
    let origin = site_origin(base_url)?;
    let parsed = Url::parse(&origin).ok()?;
    if parsed.scheme() != "https" || parsed.host_str() != Some("opencode.ai") {
        return None;
    }
    Some(format!("{}/zen/go/v1/usage", strip_trailing_slash(&origin)))
}

#[cfg(test)]
thread_local! {
    /// Test seam so the OpenCode Go probe can target a local mock server.
    static OPENCODE_GO_URL_OVERRIDE: std::cell::RefCell<Option<String>> =
        const { std::cell::RefCell::new(None) };
}

/// 魔搭余额端点。它与推理端点不同域（`modelscope.cn` vs `api-inference.modelscope.cn`），
/// 所以写成常量而不是从 `base_url` 派生 —— 派生等于把用户可控的 host 拼进带 Bearer 的请求。
pub const MAGICUBE_BALANCE_URL: &str = "https://modelscope.cn/openapi/v1/magicubes/balance";

/// 魔搭渠道识别：只接受 https 且 host 严格等于推理域名，避免
/// `api-inference.modelscope.cn.evil.com` 这类形似 host 误判进专用探测链。
/// 口径与前端 `src/lib/siteProviderKinds.ts` 的 `isModelScopeBase` 一致。
pub fn is_modelscope_base(base_url: &str) -> bool {
    let Ok(parsed) = Url::parse(base_url.trim()) else {
        return false;
    };
    parsed.scheme() == "https" && parsed.host_str() == Some("api-inference.modelscope.cn")
}

/// 空 key 时仍要发出探测的专用渠道。放行是为了让它们落到各自的 401 语义，
/// 而不是在命令层被 `empty_key_result()` 短路成「无数据」—— 那样用户看不到
/// 「令牌无效」这条可诊断的错误。
pub fn allows_empty_key_probe(base_url: &str) -> bool {
    is_opencode_go_base(base_url) || is_modelscope_base(base_url)
}

fn magicube_balance_url() -> String {
    #[cfg(test)]
    if let Some(url) = MAGICUBE_URL_OVERRIDE.with(|cell| cell.borrow().clone()) {
        return url;
    }
    MAGICUBE_BALANCE_URL.to_string()
}

#[cfg(test)]
thread_local! {
    /// Test seam so the magicube probe can target a local mock server.
    static MAGICUBE_URL_OVERRIDE: std::cell::RefCell<Option<String>> =
        const { std::cell::RefCell::new(None) };
}

pub fn normalize_quota_unit(raw: &str) -> String {
    match raw.trim().to_ascii_uppercase().as_str() {
        "RMB" | "CNY" | "¥" | "元" => "CNY".into(),
        "USD" | "$" => "USD".into(),
        other if other.is_empty() => "USD".into(),
        other => other.to_string(),
    }
}

pub fn usage_to_usd(total_usage: f64, limit: Option<f64>) -> f64 {
    let scaled = total_usage / 100.0;
    if let Some(limit) = limit {
        if limit > 0.0 && scaled > limit * 1.5 && (0.0..=limit * 1.5).contains(&total_usage) {
            return total_usage;
        }
    }
    scaled
}

fn json_f64(value: &Value) -> Option<f64> {
    match value {
        Value::Number(n) => n.as_f64().filter(|x| x.is_finite()),
        Value::String(s) => s.trim().parse::<f64>().ok().filter(|x| x.is_finite()),
        _ => None,
    }
}

fn json_i64(value: &Value) -> Option<i64> {
    match value {
        Value::Number(n) => n
            .as_i64()
            .or_else(|| n.as_f64().filter(|x| x.is_finite()).map(|f| f as i64)),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn field_f64(obj: &Value, key: &str) -> Option<f64> {
    obj.get(key).and_then(json_f64)
}

pub fn parse_credit_grants(value: &Value) -> Option<Grants> {
    if !value.is_object() {
        return None;
    }
    let remaining = field_f64(value, "total_available");
    let used = field_f64(value, "total_used");
    let total = field_f64(value, "total_granted");
    if remaining.is_none() && used.is_none() && total.is_none() {
        return None;
    }
    let mut grants = Grants {
        remaining,
        used,
        total,
    };
    if grants.remaining.is_none() {
        if let (Some(t), Some(u)) = (grants.total, grants.used) {
            grants.remaining = Some(t - u);
        }
    }
    if grants.used.is_none() {
        if let (Some(t), Some(r)) = (grants.total, grants.remaining) {
            grants.used = Some(t - r);
        }
    }
    Some(grants)
}

pub fn parse_subscription(value: &Value) -> Option<Subscription> {
    if !value.is_object() {
        return None;
    }
    let limit_usd = field_f64(value, "hard_limit_usd")
        .or_else(|| field_f64(value, "system_hard_limit_usd"))
        .or_else(|| field_f64(value, "soft_limit_usd"));
    let expires_at = value
        .get("access_until")
        .and_then(json_i64)
        .filter(|v| *v > 0);
    if limit_usd.is_none() && expires_at.is_none() {
        return None;
    }
    Some(Subscription {
        limit_usd,
        expires_at,
    })
}

pub fn parse_usage(value: &Value) -> Option<Usage> {
    if !value.is_object() {
        return None;
    }
    field_f64(value, "total_usage").map(|total_usage| Usage { total_usage })
}

pub fn parse_token_usage(value: &Value) -> Option<TokenUsage> {
    if !value.is_object() {
        return None;
    }
    if value.get("code").and_then(Value::as_bool) == Some(false) {
        return None;
    }
    if value.get("success").and_then(Value::as_bool) == Some(false) {
        return None;
    }
    let data = value.get("data").filter(|d| d.is_object())?;
    let display = data.get("display").filter(|d| d.is_object()).and_then(|d| {
        Some((
            field_f64(d, "remaining")?,
            field_f64(d, "used")?,
            field_f64(d, "total")?,
            normalize_quota_unit(d.get("unit")?.as_str()?),
        ))
    });
    let (remaining, used, total, unit, has_display) = match display {
        Some((remaining, used, total, unit)) => {
            (Some(remaining), Some(used), Some(total), unit, true)
        }
        None => (
            field_f64(data, "total_available"),
            field_f64(data, "total_used"),
            field_f64(data, "total_granted"),
            "quota".into(),
            false,
        ),
    };
    if remaining.is_none() && used.is_none() && total.is_none() {
        return None;
    }
    let flag = data
        .get("unlimited_quota")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let unlimited = if has_display { false } else { flag };
    let expires_at = data.get("expires_at").and_then(json_i64).filter(|v| *v > 0);
    Some(TokenUsage {
        remaining,
        used,
        total,
        unit,
        unlimited,
        expires_at,
        has_display,
    })
}

fn parse_quota_display_type(data: &Value) -> Option<QuotaDisplayType> {
    if let Some(raw) = data.get("quota_display_type").and_then(Value::as_str) {
        return match raw.trim().to_ascii_uppercase().as_str() {
            "USD" => Some(QuotaDisplayType::Usd),
            "CNY" => Some(QuotaDisplayType::Cny),
            "TOKENS" => Some(QuotaDisplayType::Tokens),
            "CUSTOM" => Some(QuotaDisplayType::Custom),
            _ => None,
        };
    }
    data.get("display_in_currency")
        .and_then(Value::as_bool)
        .map(|enabled| {
            if enabled {
                QuotaDisplayType::Usd
            } else {
                QuotaDisplayType::Tokens
            }
        })
}

pub fn parse_quota_status(value: &Value) -> Option<QuotaStatus> {
    if value.get("success").and_then(Value::as_bool) == Some(false) {
        return None;
    }
    let data = value.get("data").filter(|data| data.is_object())?;
    let display_type = parse_quota_display_type(data).unwrap_or(QuotaDisplayType::Raw);
    Some(QuotaStatus {
        quota_per_unit: field_f64(data, "quota_per_unit").filter(|value| *value > 0.0),
        display_type,
        usd_exchange_rate: field_f64(data, "usd_exchange_rate").filter(|value| *value > 0.0),
        custom_currency_symbol: data
            .get("custom_currency_symbol")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        custom_currency_exchange_rate: field_f64(data, "custom_currency_exchange_rate")
            .filter(|value| *value > 0.0),
    })
}

/// 解析 Sub2API `/v1/usage` 的钱包余额。
///
/// 规则（按现网实测字段，只覆盖钱包模式）：
/// - `isValid: false` 或上游失败/`error` 结构 → `None`（不得当作可用余额）；
/// - `remaining` 缺失时回退 `balance`（实测同值），都为非数 → `None`；
/// - `unit` 缺省 `"USD"`；
/// - 钱包模式没有可靠的 `used` / `total` 语义，一律留空——**不要**把 `balance`
///   冒充 `total`，否则前端会算出 0% 进度条或误导性的「已用 0」；
/// - `mode == "unrestricted"` 下 `remaining < 0` 是「不限额」哨兵，不是欠费。
pub fn parse_sub2api_usage(value: &Value) -> Option<Sub2ApiUsage> {
    if !value.is_object() {
        return None;
    }
    if value.get("isValid").and_then(Value::as_bool) == Some(false)
        || value.get("is_valid").and_then(Value::as_bool) == Some(false)
    {
        return None;
    }
    if response_indicates_failure(value) {
        return None;
    }
    let unit = value
        .get("unit")
        .and_then(Value::as_str)
        .map(normalize_quota_unit)
        .unwrap_or_else(|| "USD".into());
    let remaining = field_f64(value, "remaining").or_else(|| field_f64(value, "balance"));
    let unrestricted = value
        .get("mode")
        .and_then(Value::as_str)
        .is_some_and(|mode| mode.trim().eq_ignore_ascii_case("unrestricted"));
    if unrestricted && remaining.is_some_and(|remaining| remaining < 0.0) {
        return Some(Sub2ApiUsage {
            remaining: None,
            used: None,
            total: None,
            unit,
            unlimited: true,
        });
    }
    let remaining = remaining?;
    Some(Sub2ApiUsage {
        remaining: Some(remaining),
        used: None,
        total: None,
        unit,
        unlimited: false,
    })
}

const OPENCODE_GO_WINDOW_KINDS: [&str; 3] = ["rolling", "weekly", "monthly"];

fn opencode_go_window_keys(kind: &str) -> [&'static str; 3] {
    match kind {
        "rolling" => ["rollingUsage", "rolling_usage", "rolling"],
        "weekly" => ["weeklyUsage", "weekly_usage", "weekly"],
        _ => ["monthlyUsage", "monthly_usage", "monthly"],
    }
}

/// Find one window object inside a container, tolerating camelCase / snake_case /
/// bare kind keys (e.g. `windows.rolling`).
fn opencode_go_window_in<'a>(container: &'a Value, kind: &str) -> Option<&'a Value> {
    if !container.is_object() {
        return None;
    }
    for key in opencode_go_window_keys(kind) {
        if let Some(value) = container.get(key) {
            if value.is_object() {
                return Some(value);
            }
        }
    }
    None
}

fn opencode_go_window_object<'a>(root: &'a Value, kind: &str) -> Option<&'a Value> {
    if let Some(found) = opencode_go_window_in(root, kind) {
        return Some(found);
    }
    if let Some(found) = root
        .get("windows")
        .and_then(|container| opencode_go_window_in(container, kind))
    {
        return Some(found);
    }
    if let Some(found) = root
        .get("data")
        .and_then(|container| opencode_go_window_in(container, kind))
    {
        return Some(found);
    }
    if let Some(found) = root
        .get("data")
        .and_then(|data| data.get("windows"))
        .and_then(|container| opencode_go_window_in(container, kind))
    {
        return Some(found);
    }
    root.get("usage")
        .and_then(|container| opencode_go_window_in(container, kind))
}

/// Normalize an upstream absolute reset value to milliseconds since epoch.
/// Values below the ms threshold are treated as epoch seconds.
fn absolute_reset_ms(raw: i64) -> i64 {
    if raw >= 100_000_000_000 {
        raw
    } else {
        raw.saturating_mul(1000)
    }
}

/// Parse an RFC3339 / ISO-8601 reset timestamp into milliseconds since epoch.
/// Returns `None` on any parse failure (never panics, never coerces to 0).
fn parse_rfc3339_ms(raw: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(raw.trim())
        .ok()
        .map(|value| value.timestamp_millis())
}

fn parse_opencode_go_window(kind: &str, obj: &Value, fetched_at: i64) -> Option<QuotaWindow> {
    let usage_percent = field_f64(obj, "usagePercent")
        .or_else(|| field_f64(obj, "usage_percent"))
        .or_else(|| field_f64(obj, "percent"))
        .or_else(|| field_f64(obj, "usedPercent"))
        .or_else(|| field_f64(obj, "used_percent"));

    // 生产响应（formatUsage）用 RFC3339 字符串 `resetsAt`；数值/相对秒数为兼容回退。
    let rfc3339_reset = obj
        .get("resetsAt")
        .and_then(Value::as_str)
        .or_else(|| obj.get("resets_at").and_then(Value::as_str))
        .and_then(parse_rfc3339_ms);
    let relative_secs = obj
        .get("resetInSec")
        .and_then(json_i64)
        .or_else(|| obj.get("reset_in_sec").and_then(json_i64))
        .or_else(|| obj.get("resetInSeconds").and_then(json_i64))
        .or_else(|| obj.get("reset_in_seconds").and_then(json_i64))
        .or_else(|| obj.get("resetsInSeconds").and_then(json_i64))
        .or_else(|| obj.get("resets_in_seconds").and_then(json_i64));
    let reset_at = rfc3339_reset
        .or_else(|| {
            relative_secs.map(|secs| fetched_at.saturating_add(secs.saturating_mul(1000)))
        })
        .or_else(|| {
            obj.get("resetAt")
                .and_then(json_i64)
                .map(absolute_reset_ms)
        })
        .or_else(|| {
            obj.get("reset_at")
                .and_then(json_i64)
                .map(absolute_reset_ms)
        });

    let limit_usd = field_f64(obj, "limitUsd")
        .or_else(|| field_f64(obj, "limit_usd"))
        .or_else(|| field_f64(obj, "limit"));

    if usage_percent.is_none() && reset_at.is_none() && limit_usd.is_none() {
        return None;
    }
    Some(QuotaWindow {
        kind: kind.to_string(),
        usage_percent,
        reset_at,
        limit_usd,
    })
}

/// Parse the three OpenCode Go usage windows from a `/zen/go/v1/usage` body.
/// Relative reset seconds are converted to absolute milliseconds using `fetched_at`.
pub fn parse_opencode_go_windows(value: &Value, fetched_at: i64) -> Option<Vec<QuotaWindow>> {
    if !value.is_object() {
        return None;
    }
    let mut windows = Vec::new();
    for kind in OPENCODE_GO_WINDOW_KINDS {
        if let Some(obj) = opencode_go_window_object(value, kind) {
            if let Some(window) = parse_opencode_go_window(kind, obj, fetched_at) {
                windows.push(window);
            }
        }
    }
    if windows.is_empty() {
        None
    } else {
        Some(windows)
    }
}

/// 把 `/api/status` 的展示类型映射为 **token 额度** 的乘数 `(scale, unit)`：
/// 调用方执行 `quota * scale`（乘法）。
///
/// 账户余额不是这个语义，必须用 [`account_balance_scale`]（除数）。
fn token_quota_scale(status: &QuotaStatus) -> Option<(f64, String)> {
    match status.display_type {
        QuotaDisplayType::Tokens => Some((1.0, "TOKENS".into())),
        QuotaDisplayType::Usd => status
            .quota_per_unit
            .map(|quota_per_unit| (1.0 / quota_per_unit, "USD".into())),
        QuotaDisplayType::Cny => status
            .quota_per_unit
            .zip(status.usd_exchange_rate)
            .map(|(quota_per_unit, rate)| (rate / quota_per_unit, "CNY".into())),
        QuotaDisplayType::Custom => status
            .quota_per_unit
            .zip(status.custom_currency_exchange_rate)
            .map(|(quota_per_unit, rate)| {
                (
                    rate / quota_per_unit,
                    status
                        .custom_currency_symbol
                        .clone()
                        .unwrap_or_else(|| "CUSTOM".into()),
                )
            }),
        QuotaDisplayType::Raw => None,
    }
}

/// 默认换算倍率：new-api 的 `quota` 字段以 500000 个 quota 记 1 单位货币。
const NEWAPI_QUOTA_PER_UNIT: f64 = 500_000.0;

/// 把 `/api/status` 的展示类型映射为 **账户余额** 的除数 `(divisor, unit)`：
/// 调用方执行 `quota / divisor`（除法），与 [`token_quota_scale`] 的乘法语义相反。
///
/// `/api/status` 缺失或字段不完整从来不是错误：`None` 表示用默认
/// 500000 / USD 兜底，账户余额必须照常显示（`/api/status` 是锦上添花）。
fn account_balance_scale(status: Option<&QuotaStatus>) -> (f64, String) {
    let Some(status) = status else {
        return (NEWAPI_QUOTA_PER_UNIT, "USD".into());
    };
    let divisor = status
        .quota_per_unit
        .filter(|value| *value > 0.0)
        .unwrap_or(NEWAPI_QUOTA_PER_UNIT);
    let unit = match status.display_type {
        QuotaDisplayType::Cny => "CNY",
        QuotaDisplayType::Custom => status
            .custom_currency_symbol
            .as_deref()
            .map(str::trim)
            .filter(|symbol| !symbol.is_empty())
            .unwrap_or("USD"),
        // AgentRouter 无 quota_display_type（解析成 Raw）时保持 USD 现状。
        QuotaDisplayType::Usd | QuotaDisplayType::Raw | QuotaDisplayType::Tokens => "USD",
    };
    (divisor, unit.to_string())
}

fn quota_values_are_consistent(
    remaining: Option<f64>,
    used: Option<f64>,
    total: Option<f64>,
    unlimited: bool,
) -> bool {
    if unlimited {
        return used.map_or(true, |value| value >= 0.0);
    }
    let values = [remaining, used, total];
    if values.into_iter().flatten().any(|value| value < 0.0) {
        return false;
    }
    if let Some(total) = total {
        let tolerance = total.abs() * 1e-6 + 1e-6;
        if used.is_some_and(|used| used - total > tolerance)
            || remaining.is_some_and(|remaining| remaining - total > tolerance)
        {
            return false;
        }
    }
    let (Some(remaining), Some(used), Some(total)) = (remaining, used, total) else {
        return true;
    };
    let tolerance = total.abs().max(remaining.abs()).max(used.abs()) * 1e-6 + 1e-6;
    (total - remaining - used).abs() <= tolerance
}

pub fn normalize_token_usage(
    mut token: TokenUsage,
    status: Option<&QuotaStatus>,
) -> Result<TokenUsage, String> {
    if !token.has_display {
        let (scale, unit) = status
            .and_then(token_quota_scale)
            .unwrap_or_else(|| (1.0, "RAW_QUOTA".into()));
        token.remaining = token.remaining.map(|value| value * scale);
        token.used = token.used.map(|value| value * scale);
        token.total = token.total.map(|value| value * scale);
        token.unit = unit;
    }
    if !quota_values_are_consistent(token.remaining, token.used, token.total, token.unlimited) {
        return Err("inconsistent quota data".into());
    }
    Ok(token)
}

fn normalize_token_hit(token: &Hit, status: &Hit) -> Hit {
    let Hit::Token(token) = token else {
        return token.clone();
    };
    let metadata = match status {
        Hit::Status(metadata) => Some(metadata),
        _ => None,
    };
    match normalize_token_usage(token.clone(), metadata) {
        Ok(token) => Hit::Token(token),
        Err(error) => Hit::InvalidData(error),
    }
}

fn looks_like_html(body: &str) -> bool {
    let trimmed = body.trim_start().to_ascii_lowercase();
    trimmed.starts_with("<!doctype") || trimmed.starts_with("<html")
}

fn token_response_is_unauthorized(value: &Value) -> bool {
    let failed = value.get("success").and_then(Value::as_bool) == Some(false)
        || value.get("code").and_then(Value::as_bool) == Some(false);
    if !failed {
        return false;
    }
    let message = value
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    [
        "authorization",
        "bearer",
        "token not found",
        "invalid token",
        "unauthorized",
        "forbidden",
    ]
    .iter()
    .any(|fragment| message.contains(fragment))
}

fn response_indicates_failure(value: &Value) -> bool {
    let failed_flag = value.get("success").and_then(Value::as_bool) == Some(false)
        || value.get("code").and_then(Value::as_bool) == Some(false);
    let has_error = value.get("error").is_some_and(|error| match error {
        Value::Null => false,
        Value::String(message) => !message.trim().is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(fields) => !fields.is_empty(),
        Value::Bool(value) => *value,
        Value::Number(_) => true,
    });
    failed_flag || has_error
}

pub fn classify_status(status: u16, body: &str, expected: Expected) -> Hit {
    if status == 408 {
        return Hit::Error("request timed out".into());
    }
    if status == 401 {
        return Hit::Unauthorized;
    }
    if status == 403 {
        if looks_like_html(body) {
            return Hit::Unsupported;
        }
        return Hit::Unauthorized;
    }
    if status == 404 || status == 405 || status == 501 {
        return Hit::NotFound;
    }
    if (200..300).contains(&status) {
        if looks_like_html(body) {
            return Hit::Unsupported;
        }
        let Ok(value) = serde_json::from_str::<Value>(body) else {
            return Hit::Unsupported;
        };
        if matches!(expected, Expected::Token) && token_response_is_unauthorized(&value) {
            return Hit::Unauthorized;
        }
        if response_indicates_failure(&value) {
            return Hit::Error("upstream response indicated failure".into());
        }
        return match expected {
            Expected::Grants => parse_credit_grants(&value)
                .map(Hit::Grants)
                .unwrap_or(Hit::Unsupported),
            Expected::Subscription => parse_subscription(&value)
                .map(Hit::Subscription)
                .unwrap_or(Hit::Unsupported),
            Expected::Usage => parse_usage(&value)
                .map(Hit::Usage)
                .unwrap_or(Hit::Unsupported),
            Expected::Token => match parse_token_usage(&value) {
                Some(token) => Hit::Token(token),
                None if value.get("data").is_some_and(Value::is_object) => {
                    Hit::InvalidData("invalid quota data".into())
                }
                None => Hit::Unsupported,
            },
            Expected::Status => parse_quota_status(&value)
                .map(Hit::Status)
                .unwrap_or(Hit::Unsupported),
            Expected::Sub2ApiUsage => parse_sub2api_usage(&value)
                .map(Hit::Sub2ApiUsage)
                .unwrap_or(Hit::Unsupported),
        };
    }
    Hit::Error(format!("HTTP {status}"))
}

#[derive(Debug, Clone, Copy)]
pub enum Expected {
    Grants,
    Subscription,
    Usage,
    Token,
    Status,
    /// Sub2API `/v1/usage` 钱包余额；不可用时静默降级为 `Unsupported`。
    Sub2ApiUsage,
}

fn clamp_remaining(value: Option<f64>) -> Option<f64> {
    value.map(|v| if v.is_finite() { v.max(0.0) } else { 0.0 })
}

fn quiet(
    status: QuotaProbeStatus,
    error: Option<String>,
    latency_ms: u64,
    endpoint: Option<String>,
) -> SiteQuota {
    SiteQuota {
        status,
        remaining_usd: None,
        used_usd: None,
        total_usd: None,
        unlimited: false,
        unit: None,
        expires_at: None,
        source: None,
        endpoint,
        fetched_at: Utc::now().timestamp_millis(),
        latency_ms,
        error,
        windows: Vec::new(),
    }
}

fn available(
    source: QuotaSource,
    remaining: Option<f64>,
    used: Option<f64>,
    total: Option<f64>,
    unlimited: bool,
    unit: Option<&str>,
    expires_at: Option<i64>,
    endpoint: Option<String>,
    fetched_at: i64,
    latency_ms: u64,
) -> Result<SiteQuota, String> {
    if !quota_values_are_consistent(remaining, used, total, unlimited) {
        return Err("inconsistent quota data".into());
    }
    Ok(SiteQuota {
        status: QuotaProbeStatus::Available,
        remaining_usd: if unlimited {
            None
        } else {
            clamp_remaining(remaining)
        },
        used_usd: used,
        total_usd: if unlimited { None } else { total },
        unlimited,
        unit: unit.map(|s| s.to_string()),
        expires_at,
        source: Some(source),
        endpoint,
        fetched_at,
        latency_ms,
        error: None,
        windows: Vec::new(),
    })
}

pub fn interpret_round(
    grants: &Hit,
    subscription: &Hit,
    usage: &Hit,
    token: &Hit,
    grants_url: &str,
    subscription_url: &str,
    usage_url: &str,
    token_url: &str,
    fetched_at: i64,
    latency_ms: u64,
    allow_fallback: bool,
) -> RoundOutcome {
    let expires = match subscription {
        Hit::Subscription(s) => s.expires_at,
        _ => None,
    };
    let mut invalid = match token {
        Hit::InvalidData(error) => Some((error.clone(), token_url.to_string())),
        _ => None,
    };

    if let Hit::Token(t) = token {
        match available(
            QuotaSource::TokenUsage,
            t.remaining,
            t.used,
            t.total,
            t.unlimited,
            Some(&t.unit),
            t.expires_at.or(expires),
            Some(token_url.to_string()),
            fetched_at,
            latency_ms,
        ) {
            Ok(quota) => return RoundOutcome::Available(quota),
            Err(error) => invalid = Some((error, token_url.to_string())),
        }
    }

    if let Hit::Grants(g) = grants {
        match available(
            QuotaSource::CreditGrants,
            g.remaining,
            g.used,
            g.total,
            false,
            Some("USD"),
            expires,
            Some(grants_url.to_string()),
            fetched_at,
            latency_ms,
        ) {
            Ok(quota) => return RoundOutcome::Available(quota),
            Err(error) => {
                invalid.get_or_insert((error, grants_url.to_string()));
            }
        }
    }

    let sub = match subscription {
        Hit::Subscription(s) => Some(s),
        _ => None,
    };
    let usg = match usage {
        Hit::Usage(u) => Some(u),
        _ => None,
    };

    if let (Some(s), Some(u)) = (sub, usg) {
        let used = usage_to_usd(u.total_usage, s.limit_usd);
        let remaining = s.limit_usd.map(|limit| limit - used);
        match available(
            QuotaSource::SubscriptionUsage,
            remaining,
            Some(used),
            s.limit_usd,
            false,
            Some("USD"),
            s.expires_at,
            Some(subscription_url.to_string()),
            fetched_at,
            latency_ms,
        ) {
            Ok(quota) => return RoundOutcome::Available(quota),
            Err(error) => {
                invalid.get_or_insert((error, subscription_url.to_string()));
            }
        }
    } else if let Some(s) = sub.filter(|subscription| subscription.limit_usd.is_some()) {
        match available(
            QuotaSource::SubscriptionOnly,
            None,
            None,
            s.limit_usd,
            false,
            Some("USD"),
            s.expires_at,
            Some(subscription_url.to_string()),
            fetched_at,
            latency_ms,
        ) {
            Ok(quota) => return RoundOutcome::Available(quota),
            Err(error) => {
                invalid.get_or_insert((error, subscription_url.to_string()));
            }
        }
    } else if let Some(u) = usg {
        let used = usage_to_usd(u.total_usage, None);
        match available(
            QuotaSource::UsageOnly,
            None,
            Some(used),
            None,
            false,
            Some("USD"),
            expires,
            Some(usage_url.to_string()),
            fetched_at,
            latency_ms,
        ) {
            Ok(quota) => return RoundOutcome::Available(quota),
            Err(error) => {
                invalid.get_or_insert((error, usage_url.to_string()));
            }
        }
    }

    let hits = [grants, subscription, usage, token];
    if allow_fallback {
        return RoundOutcome::Fallback;
    }
    if let Some((error, endpoint)) = invalid {
        return RoundOutcome::Quiet(quiet(
            QuotaProbeStatus::InvalidData,
            Some(error),
            latency_ms,
            Some(endpoint),
        ));
    }
    if hits.iter().any(|h| matches!(h, Hit::Unauthorized)) {
        return RoundOutcome::Quiet(quiet(
            QuotaProbeStatus::Unauthorized,
            None,
            latency_ms,
            None,
        ));
    }
    if let Some(Hit::Error(msg)) = hits.iter().find(|h| matches!(h, Hit::Error(_))) {
        return RoundOutcome::Quiet(quiet(
            QuotaProbeStatus::Error,
            Some(msg.clone()),
            latency_ms,
            None,
        ));
    }
    RoundOutcome::Quiet(quiet(QuotaProbeStatus::Unsupported, None, latency_ms, None))
}

fn quota_source_rank(source: Option<QuotaSource>) -> u8 {
    match source {
        Some(QuotaSource::OpencodeGo) => 6,
        // 专用渠道在 `probe_quota` 里就早退了，永远不进这里的合并；给个高值只为
        // 保持穷尽，不改变任何既有排序。
        Some(QuotaSource::MagicubeBalance) => 6,
        Some(QuotaSource::TokenUsage) => 5,
        Some(QuotaSource::UserSelf) => 5,
        Some(QuotaSource::CreditGrants) => 4,
        // Sub2API 钱包余额与 credit grants 同级；它只在标准链判定「不支持」后才会
        // 产生，因此这里的排序永远不会抢走既有来源。
        Some(QuotaSource::Sub2Api) => 4,
        Some(QuotaSource::SubscriptionUsage) => 3,
        Some(QuotaSource::SubscriptionOnly) => 2,
        Some(QuotaSource::UsageOnly) => 1,
        None => 0,
    }
}

fn quota_status_rank(status: QuotaProbeStatus) -> u8 {
    match status {
        QuotaProbeStatus::InvalidData => 4,
        QuotaProbeStatus::Unauthorized => 3,
        QuotaProbeStatus::Error => 2,
        QuotaProbeStatus::Unsupported => 1,
        QuotaProbeStatus::Available => 0,
    }
}

pub fn combine_round_outcomes(first: RoundOutcome, second: RoundOutcome) -> RoundOutcome {
    match (first, second) {
        (RoundOutcome::Fallback, outcome) | (outcome, RoundOutcome::Fallback) => outcome,
        (RoundOutcome::Available(first), RoundOutcome::Available(second)) => {
            if quota_source_rank(first.source) >= quota_source_rank(second.source) {
                RoundOutcome::Available(first)
            } else {
                RoundOutcome::Available(second)
            }
        }
        (RoundOutcome::Available(quota), RoundOutcome::Quiet(_))
        | (RoundOutcome::Quiet(_), RoundOutcome::Available(quota)) => {
            RoundOutcome::Available(quota)
        }
        (RoundOutcome::Quiet(first), RoundOutcome::Quiet(second)) => {
            if quota_status_rank(first.status) >= quota_status_rank(second.status) {
                RoundOutcome::Quiet(first)
            } else {
                RoundOutcome::Quiet(second)
            }
        }
    }
}

fn outcome_needs_fallback(outcome: &RoundOutcome) -> bool {
    match outcome {
        RoundOutcome::Available(quota) => {
            quota_source_rank(quota.source) < quota_source_rank(Some(QuotaSource::CreditGrants))
        }
        RoundOutcome::Quiet(_) | RoundOutcome::Fallback => true,
    }
}

async fn fetch_hit(
    client: &reqwest::Client,
    url: &str,
    api_key: &str,
    expected: Expected,
    authenticated: bool,
) -> Hit {
    let request = client.get(url).header("Accept", "application/json");
    let request = if authenticated {
        request.bearer_auth(api_key)
    } else {
        request
    };
    match request.send().await {
        Ok(resp) => {
            let status = resp.status().as_u16();
            let bytes = match resp.bytes().await {
                Ok(bytes) => bytes,
                Err(error) => return Hit::Error(sanitize_error(&error.to_string(), api_key)),
            };
            let slice = if bytes.len() > MAX_BODY_BYTES {
                &bytes[..MAX_BODY_BYTES]
            } else {
                &bytes
            };
            let text = String::from_utf8_lossy(slice).into_owned();
            classify_status(status, &text, expected)
        }
        Err(err) => {
            let msg = if err.is_timeout() {
                "request timed out".to_string()
            } else {
                err.to_string()
            };
            Hit::Error(sanitize_error(&msg, api_key))
        }
    }
}

async fn fetch_public_pair(
    client: &reqwest::Client,
    bases: &[String],
    api_key: &str,
) -> (Hit, String, Hit) {
    for (index, base) in bases.iter().enumerate() {
        let status_url = quota_status_url(base);
        let status = fetch_hit(client, &status_url, api_key, Expected::Status, false).await;
        if index > 0 && !matches!(status, Hit::Status(_)) {
            continue;
        }

        let token_url = token_usage_url(base);
        let token = fetch_hit(client, &token_url, api_key, Expected::Token, true).await;
        match token {
            Hit::Token(_) | Hit::Unauthorized | Hit::Error(_) => {
                return (token, token_url, status);
            }
            Hit::NotFound | Hit::Unsupported => {}
            _ => return (token, token_url, status),
        }
    }
    (Hit::Unsupported, String::new(), Hit::Unsupported)
}

struct ProbeRound {
    urls: BillingUrls,
    token_url: String,
    grants: Hit,
    subscription: Hit,
    usage: Hit,
    token: Hit,
    status: Hit,
}

async fn fetch_round(
    client: &reqwest::Client,
    api_root: &str,
    public_bases: &[String],
    api_key: &str,
    include_public: bool,
) -> ProbeRound {
    let urls = billing_urls(api_root, Utc::now().date_naive());
    let public_fut = async {
        if include_public {
            fetch_public_pair(client, public_bases, api_key).await
        } else {
            (Hit::Unsupported, String::new(), Hit::Unsupported)
        }
    };
    let (grants, subscription, usage, (token, token_url, status)) = tokio::join!(
        fetch_hit(client, &urls.credit_grants, api_key, Expected::Grants, true),
        fetch_hit(
            client,
            &urls.subscription,
            api_key,
            Expected::Subscription,
            true
        ),
        fetch_hit(client, &urls.usage, api_key, Expected::Usage, true),
        public_fut,
    );
    ProbeRound {
        urls,
        token_url,
        grants,
        subscription,
        usage,
        token,
        status,
    }
}

fn finish_round(
    round: &ProbeRound,
    start: Instant,
    fetched_at: i64,
    api_key: &str,
    allow_fallback: bool,
) -> RoundOutcome {
    let latency_ms = start.elapsed().as_millis() as u64;
    let token = normalize_token_hit(&round.token, &round.status);
    let outcome = interpret_round(
        &round.grants,
        &round.subscription,
        &round.usage,
        &token,
        &round.urls.credit_grants,
        &round.urls.subscription,
        &round.urls.usage,
        &round.token_url,
        fetched_at,
        latency_ms,
        allow_fallback,
    );
    match outcome {
        RoundOutcome::Quiet(mut q) if q.status == QuotaProbeStatus::Error => {
            if let Some(err) = q.error.as_mut() {
                *err = sanitize_error(err, api_key);
            }
            RoundOutcome::Quiet(q)
        }
        other => other,
    }
}

/// NewAPI 访问令牌连通性测试结果（供站点编辑里的「测试」按钮使用）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NewApiAccessProbe {
    pub ok: bool,
    pub status: u16,
    pub remaining_usd: Option<f64>,
    pub used_usd: Option<f64>,
    pub total_usd: Option<f64>,
    /// 账户余额的货币单位，与主界面额度行同源（站点自报 `quota_display_type`）；
    /// 失败或拿不到换算参数时为 `None`。
    pub unit: Option<String>,
    pub endpoint: String,
    pub message: Option<String>,
}

fn truncate_message(body: &str, token: &str) -> String {
    let sanitized = sanitize_error(body, token);
    let trimmed = sanitized.trim();
    if trimmed.chars().count() > 200 {
        trimmed.chars().take(200).collect::<String>() + "…"
    } else {
        trimmed.to_string()
    }
}

/// 尽力而为地取同一 origin 的 `/api/status` 换算参数。任何失败（网络/超时/非 2xx/
/// 非法 JSON/字段缺失）都只返回 `None`，由 [`account_balance_scale`] 回退默认倍率；
/// `/api/status` 是锦上添花，绝不能因为拿不到它而让账户余额整体失败。
async fn request_quota_status(client: &reqwest::Client, origin: &str) -> Option<QuotaStatus> {
    match fetch_hit(
        client,
        &quota_status_url(origin),
        "",
        Expected::Status,
        false,
    )
    .await
    {
        Hit::Status(status) => Some(status),
        _ => None,
    }
}

/// `/api/user/self` 的单次请求结果。站点编辑里的「测试」按钮与主界面额度行
/// 共用这一条请求/解析链路，保证两处显示的账户余额来源完全一致。
struct UserSelfResponse {
    status: u16,
    body: String,
    endpoint: String,
    /// 成功且未被上游标记为失败时的 `data` 对象。
    data: Option<Value>,
    /// 同一 origin 自报的换算参数；拿不到即 `None`（回退 500000/USD）。
    quota_status: Option<QuotaStatus>,
}

/// 请求 `/api/user/self` 的同时**并发**取同一 origin 的 `/api/status`（换算参数），
/// 避免串行翻倍延迟。status 为 `Option` 语义：失败只是回退默认倍率。
async fn request_user_self(
    client: &reqwest::Client,
    origin: &str,
    token: &str,
    user_id: &str,
) -> reqwest::Result<UserSelfResponse> {
    let endpoint = format!("{}/api/user/self", strip_trailing_slash(origin));
    let (response, quota_status) = tokio::join!(
        async {
            let resp = client
                .get(&endpoint)
                .bearer_auth(token)
                .header("New-Api-User", user_id)
                .send()
                .await?;
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            let data = serde_json::from_str::<Value>(&body)
                .ok()
                .and_then(|value| {
                    if response_indicates_failure(&value) {
                        None
                    } else {
                        value.get("data").filter(|data| data.is_object()).cloned()
                    }
                });
            Ok::<_, reqwest::Error>(UserSelfResponse {
                status,
                body,
                endpoint: endpoint.clone(),
                data,
                quota_status: None,
            })
        },
        request_quota_status(client, origin),
    );
    let mut response = response?;
    response.quota_status = quota_status;
    Ok(response)
}

/// `/api/user/self` 的候选 origin：与标准探测链共用同一套候选（先去 `/v1`
/// 得到路径前缀 origin，再回退站点根）。带路径前缀的 Base URL（如
/// `https://host/openai/v1`）只有回退到站点根才命中，因此「测试」按钮与主界面
/// 额度行必须遍历同一个集合，否则两处会取到不同来源的余额。
fn user_self_candidates(base_url: &str) -> Vec<String> {
    let mut candidates = public_api_bases(base_url);
    if candidates.is_empty() {
        candidates.push(strip_trailing_slash(base_url).to_string());
    }
    candidates
}

/// 由 `/api/user/self` 的 `data` 换算（剩余, 已用, 总额）金额：以站点 `/api/status`
/// 自报的 `quota_per_unit` 作**除数**（见 [`account_balance_scale`]），缺失或非正数
/// 时回退 500000。账户余额的换算口径只此一处，两处展示共用，避免除数或口径分叉。
fn user_self_amounts(data: &Value, status: Option<&QuotaStatus>) -> Option<(f64, f64, f64)> {
    let quota = field_f64(data, "quota")?;
    let used = field_f64(data, "used_quota").unwrap_or(0.0);
    let (divisor, _unit) = account_balance_scale(status);
    Some((
        quota / divisor,
        used / divisor,
        (quota + used) / divisor,
    ))
}

/// 判定一次请求是否给出了可信的账户余额。要求 2xx 且 `data.quota` 可解析：
/// 非 2xx 但 body 恰好可解析的响应（网关错误页等）在「测试」按钮里也必须算失败，
/// 否则会出现「测试通过、主界面回落到别的来源」的分叉。
fn user_self_success(response: &UserSelfResponse) -> Option<(f64, f64, f64)> {
    if !(200..300).contains(&response.status) {
        return None;
    }
    response
        .data
        .as_ref()
        .and_then(|data| user_self_amounts(data, response.quota_status.as_ref()))
}

/// 测试访问令牌：按候选 origin 逐个尝试 /api/user/self，返回首个非 404 的结果。
pub async fn test_newapi_access(
    base_url: &str,
    token: &str,
    user_id: &str,
    settings: &AppSettings,
) -> AppResult<NewApiAccessProbe> {
    let preview = normalize_base_url(base_url)?;
    let candidates = user_self_candidates(&preview.codex_base_url);
    let client = crate::http_client::build_client(settings, PROBE_TIMEOUT)?;
    let mut last: Option<NewApiAccessProbe> = None;
    for origin in candidates {
        let response = request_user_self(&client, &origin, token, user_id).await?;
        let status = response.status;
        let balance = user_self_success(&response);
        let url = response.endpoint;
        if status == 404 {
            last = Some(NewApiAccessProbe {
                ok: false,
                status,
                remaining_usd: None,
                used_usd: None,
                total_usd: None,
                unit: None,
                endpoint: url,
                message: Some(truncate_message(&response.body, token)),
            });
            continue;
        }
        if let Some((remaining, used, total)) = balance {
            // 单位与主界面额度行同源：站点自报的展示类型决定 USD/CNY。
            let (_divisor, unit) = account_balance_scale(response.quota_status.as_ref());
            return Ok(NewApiAccessProbe {
                ok: true,
                status,
                remaining_usd: Some(remaining),
                used_usd: Some(used),
                total_usd: Some(total),
                unit: Some(unit),
                endpoint: url,
                message: None,
            });
        }
        return Ok(NewApiAccessProbe {
            ok: false,
            status,
            remaining_usd: None,
            used_usd: None,
            total_usd: None,
            unit: None,
            endpoint: url,
            message: Some(truncate_message(&response.body, token)),
        });
    }
    Ok(last.unwrap_or(NewApiAccessProbe {
        ok: false,
        status: 404,
        remaining_usd: None,
        used_usd: None,
        total_usd: None,
        unit: None,
        endpoint: String::new(),
        message: None,
    }))
}

/// 用 NewAPI 访问令牌查账户级钱包余额（`/api/user/self`）。
/// 这是配置了访问令牌时的最高优先级来源：new-api 的 billing 上限可能是
/// 「无限额度」哨兵值，只有账户令牌链路能拿到用户真实可用的余额。
/// 候选 origin 与「测试」按钮完全一致（见 `user_self_candidates`）。
async fn fetch_user_self_quota(
    client: &reqwest::Client,
    base_url: &str,
    token: &str,
    user_id: &str,
    start: Instant,
    fetched_at: i64,
) -> Option<SiteQuota> {
    for origin in user_self_candidates(base_url) {
        let response = request_user_self(client, origin.as_str(), token, user_id)
            .await
            .ok()?;
        let Some((remaining, used, total)) = user_self_success(&response) else {
            continue;
        };
        // 货币单位与除数同源：站点自报的 `quota_display_type` 决定 USD/CNY，
        // 缺失时保持 USD（AgentRouter 现状）。
        let (_divisor, unit) = account_balance_scale(response.quota_status.as_ref());
        return Some(SiteQuota {
            status: QuotaProbeStatus::Available,
            remaining_usd: Some(remaining),
            used_usd: Some(used),
            total_usd: Some(total),
            unlimited: false,
            unit: Some(unit),
            expires_at: None,
            source: Some(QuotaSource::UserSelf),
            endpoint: Some(response.endpoint),
            fetched_at,
            latency_ms: start.elapsed().as_millis() as u64,
            error: None,
            windows: Vec::new(),
        });
    }
    None
}

/// new-api 用 1e8 量级的 `hard_limit_usd`（如 100000000）表达「无限额度」，
/// 标准探测链会把它折算成荒谬的美元余额。
///
/// 判据只看 **剩余金额**（缺失时才看总额）：已用 / 总额在累计口径下可能天然很大，
/// 若用它们触发，会把一个真实可用的剩余金额一起清掉、误报成「不限额」。
/// 阈值取 1000 万：new-api 的哨兵值远超它，而真实可花费余额不会到这个量级。
pub const ABSURD_AMOUNT_USD: f64 = 10_000_000.0;

pub fn sanitize_absurd_amounts(mut quota: SiteQuota) -> SiteQuota {
    let absurd = match quota.remaining_usd {
        Some(remaining) => remaining >= ABSURD_AMOUNT_USD,
        None => quota
            .total_usd
            .is_some_and(|total| total >= ABSURD_AMOUNT_USD),
    };
    if !absurd {
        return quota;
    }
    quota.unlimited = true;
    quota.remaining_usd = None;
    quota.used_usd = None;
    quota.total_usd = None;
    quota
}

/// Classify an OpenCode Go usage response into a `SiteQuota`.
fn opencode_go_quota(
    status: u16,
    body: &str,
    fetched_at: i64,
    latency_ms: u64,
    endpoint: &str,
) -> SiteQuota {
    let quiet_at = |probe_status: QuotaProbeStatus, error: Option<String>| {
        quiet(probe_status, error, latency_ms, Some(endpoint.to_string()))
    };
    match status {
        401 => quiet_at(QuotaProbeStatus::Unauthorized, None),
        // 403 是「key 有效但无 Go 订阅」的 EntitlementError，提示换 key 会误导。
        403 | 404 | 405 | 501 => quiet_at(QuotaProbeStatus::Unsupported, None),
        200..=299 => {
            if looks_like_html(body) {
                return quiet_at(QuotaProbeStatus::Unsupported, None);
            }
            let Ok(value) = serde_json::from_str::<Value>(body) else {
                return quiet_at(QuotaProbeStatus::Unsupported, None);
            };
            match parse_opencode_go_windows(&value, fetched_at) {
                Some(windows) if !windows.is_empty() => SiteQuota {
                    status: QuotaProbeStatus::Available,
                    remaining_usd: None,
                    used_usd: None,
                    total_usd: None,
                    unlimited: false,
                    unit: Some("USD".into()),
                    expires_at: None,
                    source: Some(QuotaSource::OpencodeGo),
                    endpoint: Some(endpoint.to_string()),
                    fetched_at,
                    latency_ms,
                    error: None,
                    windows,
                },
                _ => quiet_at(
                    QuotaProbeStatus::InvalidData,
                    Some("invalid quota data".into()),
                ),
            }
        }
        other => quiet_at(QuotaProbeStatus::Error, Some(format!("HTTP {other}"))),
    }
}

/// Fetch the fixed OpenCode Go usage endpoint and map the response.
async fn probe_opencode_go_usage(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
    start: Instant,
    fetched_at: i64,
) -> SiteQuota {
    let latency = || start.elapsed().as_millis() as u64;
    let Some(url) = opencode_go_usage_url(base_url) else {
        return quiet(
            QuotaProbeStatus::Unsupported,
            None,
            latency(),
            None,
        );
    };
    let response = client
        .get(&url)
        .bearer_auth(api_key)
        .header("Accept", "application/json")
        .send()
        .await;
    let response = match response {
        Ok(response) => response,
        Err(error) => {
            let message = if error.is_timeout() {
                "request timed out".to_string()
            } else {
                error.to_string()
            };
            return quiet(
                QuotaProbeStatus::Error,
                Some(sanitize_error(&message, api_key)),
                latency(),
                Some(url),
            );
        }
    };
    let status = response.status().as_u16();
    let bytes = match response.bytes().await {
        Ok(bytes) => bytes,
        Err(error) => {
            return quiet(
                QuotaProbeStatus::Error,
                Some(sanitize_error(&error.to_string(), api_key)),
                latency(),
                Some(url),
            );
        }
    };
    let slice = if bytes.len() > MAX_BODY_BYTES {
        &bytes[..MAX_BODY_BYTES]
    } else {
        &bytes
    };
    let body = String::from_utf8_lossy(slice).into_owned();
    opencode_go_quota(status, &body, fetched_at, latency(), &url)
}

/// 上游 `code` 只保留稳定标识字符并截断后回显：`message` 是自由文本，可能带回首
/// 请求头或令牌，因此一律不进错误串。
fn magicube_code_tag(value: &Value) -> String {
    const MAX_CODE_CHARS: usize = 64;
    let raw = match value.get("code") {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Number(number)) => number.to_string(),
        _ => String::new(),
    };
    let filtered: String = raw
        .trim()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
        .take(MAX_CODE_CHARS)
        .collect();
    if filtered.is_empty() {
        "success=false".into()
    } else {
        format!("code={filtered}")
    }
}

/// Classify a magicube balance response into a `SiteQuota`.
///
/// 口径是「魔粒账户余额」，既不是金额也不是调用次数，所以 unit 固定为 `MAGICUBE`；
/// 数据缺失或畸形一律 `invalid_data`，绝不兜底成 0 魔粒 —— 0 与「查不到」在用户
/// 眼里是完全不同的两件事。
fn magicube_quota(
    status: u16,
    body: &str,
    fetched_at: i64,
    latency_ms: u64,
    endpoint: &str,
) -> SiteQuota {
    let quiet_at = |probe_status: QuotaProbeStatus, error: Option<String>| {
        quiet(probe_status, error, latency_ms, Some(endpoint.to_string()))
    };
    let invalid = || quiet_at(QuotaProbeStatus::InvalidData, Some("invalid quota data".into()));
    match status {
        // 魔搭的 403 就是令牌没有权限（不像 Go 那样区分订阅权益），仍归认证失败。
        401 | 403 => quiet_at(QuotaProbeStatus::Unauthorized, None),
        200..=299 => {
            let Ok(value) = serde_json::from_str::<Value>(body) else {
                return invalid();
            };
            if value.get("success").and_then(Value::as_bool) == Some(false) {
                return quiet_at(QuotaProbeStatus::InvalidData, Some(magicube_code_tag(&value)));
            }
            let Some(data) = value.get("data").filter(|data| data.is_object()) else {
                return invalid();
            };
            let Some(balance) = field_f64(data, "available_balance") else {
                return invalid();
            };
            // `frozen_amount` 不显示：它需要额外解释才不会读成「已花费」。
            match available(
                QuotaSource::MagicubeBalance,
                Some(balance),
                None,
                field_f64(data, "total_balance"),
                false,
                Some("MAGICUBE"),
                None,
                Some(endpoint.to_string()),
                fetched_at,
                latency_ms,
            ) {
                Ok(quota) => quota,
                Err(_) => invalid(),
            }
        }
        other => quiet_at(QuotaProbeStatus::Error, Some(format!("HTTP {other}"))),
    }
}

/// Fetch the fixed magicube balance endpoint and map the response.
async fn probe_magicube_balance(
    client: &reqwest::Client,
    api_key: &str,
    start: Instant,
    fetched_at: i64,
) -> SiteQuota {
    let latency = || start.elapsed().as_millis() as u64;
    let url = magicube_balance_url();
    let response = match client
        .get(&url)
        .bearer_auth(api_key)
        .header("Accept", "application/json")
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) => {
            let message = if error.is_timeout() {
                "request timed out".to_string()
            } else {
                error.to_string()
            };
            return quiet(
                QuotaProbeStatus::Error,
                Some(sanitize_error(&message, api_key)),
                latency(),
                Some(url),
            );
        }
    };
    let status = response.status().as_u16();
    let bytes = match response.bytes().await {
        Ok(bytes) => bytes,
        Err(error) => {
            return quiet(
                QuotaProbeStatus::Error,
                Some(sanitize_error(&error.to_string(), api_key)),
                latency(),
                Some(url),
            );
        }
    };
    let slice = if bytes.len() > MAX_BODY_BYTES {
        &bytes[..MAX_BODY_BYTES]
    } else {
        &bytes
    };
    let body = String::from_utf8_lossy(slice).into_owned();
    magicube_quota(status, &body, fetched_at, latency(), &url)
}

/// Sub2API 钱包余额探测：按候选 origin 依次尝试 `GET {origin}/v1/usage`。
///
/// 只在标准链最终判定「不支持」之后调用，保证 new-api 站点零额外请求
/// （见 `probe_quota`）。`classify_status` 已把 404/405/501/HTML/非法 JSON 归为
/// `NotFound` / `Unsupported`；连同网络/上游错误一起安静降级（返回 `None`，
/// 维持标准链的 Unsupported），不让 new-api 站点因为多打这一枪变成错误态。
/// 只有认证失败（401/403 且非 UA 拦截）沿用既有 `Unauthorized` 语义。
async fn probe_sub2api_usage(
    client: &reqwest::Client,
    bases: &[String],
    api_key: &str,
    start: Instant,
    fetched_at: i64,
) -> Option<SiteQuota> {
    let mut unauthorized: Option<String> = None;
    for base in bases {
        let url = sub2api_usage_url(base);
        let hit = fetch_hit(client, &url, api_key, Expected::Sub2ApiUsage, true).await;
        match hit {
            Hit::Sub2ApiUsage(usage) => {
                if let Ok(quota) = available(
                    QuotaSource::Sub2Api,
                    usage.remaining,
                    usage.used,
                    usage.total,
                    usage.unlimited,
                    Some(&usage.unit),
                    None,
                    Some(url),
                    fetched_at,
                    start.elapsed().as_millis() as u64,
                ) {
                    return Some(quota);
                }
            }
            // 认证失败先记下，仍继续尝试其它候选（可用余额优先于诊断）。
            Hit::Unauthorized => {
                unauthorized.get_or_insert(url);
            }
            // 该 origin 上不是 Sub2API（404/HTML/非法 JSON/上游错误/网络错误），
            // 继续尝试下一个候选（如站点根）。
            _ => {}
        }
    }
    unauthorized.map(|url| {
        quiet(
            QuotaProbeStatus::Unauthorized,
            None,
            start.elapsed().as_millis() as u64,
            Some(url),
        )
    })
}

pub async fn probe_quota(
    site: &SiteRow,
    api_key: &str,
    settings: &AppSettings,
    newapi_creds: Option<(&str, &str)>,
) -> AppResult<SiteQuota> {
    let start = Instant::now();
    let fetched_at = Utc::now().timestamp_millis();

    // OpenCode Go 渠道只查官方用量端点，不走 billing/token/status 标准链。
    if is_opencode_go_base(&site.base_url) {
        if api_key.trim().is_empty() {
            return Ok(quiet(
                QuotaProbeStatus::Unauthorized,
                None,
                start.elapsed().as_millis() as u64,
                None,
            ));
        }
        let preview = normalize_base_url(&site.base_url)?;
        let client = crate::http_client::build_client(settings, PROBE_TIMEOUT)?;
        return Ok(
            probe_opencode_go_usage(&client, &preview.codex_base_url, api_key, start, fetched_at)
                .await,
        );
    }

    // 魔搭只查官方魔粒余额端点，同样不进标准 billing/token/status 链；凭据就用
    // 站点自己的 API Key（官方规范里推理与余额共用同一种 bearerAuth）。
    if is_modelscope_base(&site.base_url) {
        if api_key.trim().is_empty() {
            return Ok(quiet(
                QuotaProbeStatus::Unauthorized,
                None,
                start.elapsed().as_millis() as u64,
                None,
            ));
        }
        let client = crate::http_client::build_client(settings, PROBE_TIMEOUT)?;
        return Ok(probe_magicube_balance(&client, api_key, start, fetched_at).await);
    }

    if api_key.trim().is_empty() {
        return Ok(empty_key_result());
    }

    let preview = normalize_base_url(&site.base_url)?;
    let client = crate::http_client::build_client(settings, PROBE_TIMEOUT)?;

    // 来源优先级：配置了访问令牌时先查 `/api/user/self` —— 这才是账户真实余额。
    // new-api 的 billing 上限可能是「无限额度」哨兵值，标准链会算出荒谬数字。
    if let Some((token, user_id)) = newapi_creds {
        if let Some(quota) = fetch_user_self_quota(
            &client,
            &preview.codex_base_url,
            token,
            user_id,
            start,
            fetched_at,
        )
        .await
        {
            return Ok(quota);
        }
    }

    let public_bases = public_api_bases(&preview.codex_base_url);
    let round = fetch_round(
        &client,
        &preview.codex_base_url,
        &public_bases,
        api_key,
        true,
    )
    .await;
    let first = finish_round(&round, start, fetched_at, api_key, false);
    let combined = if outcome_needs_fallback(&first) {
        match origin_without_v1(&preview.codex_base_url) {
            Some(origin) if origin != preview.codex_base_url => {
                let round = fetch_round(&client, &origin, &[], api_key, false).await;
                combine_round_outcomes(
                    first,
                    finish_round(&round, start, fetched_at, api_key, false),
                )
            }
            _ => first,
        }
    } else {
        first
    };
    let quota = match combined {
        RoundOutcome::Available(quota) | RoundOutcome::Quiet(quota) => quota,
        RoundOutcome::Fallback => quiet(
            QuotaProbeStatus::Unsupported,
            None,
            start.elapsed().as_millis() as u64,
            None,
        ),
    };

    // 标准链判定「不支持」之后才给 Sub2API 一次机会：标准链命中的 new-api 站点
    // 完全不受影响（零额外请求）；AiHub 这类全 404 的站点才会多打一次 /v1/usage。
    if quota.status == QuotaProbeStatus::Unsupported {
        if let Some(sub2api) =
            probe_sub2api_usage(&client, &public_bases, api_key, start, fetched_at).await
        {
            return Ok(sanitize_absurd_amounts(sub2api));
        }
    }
    Ok(sanitize_absurd_amounts(quota))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::url_normalize::normalize_base_url;
    use serde_json::json;
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// Sets the OpenCode Go endpoint override for the current test thread.
    struct OpencodeUrlGuard;

    impl OpencodeUrlGuard {
        fn set(url: String) -> Self {
            OPENCODE_GO_URL_OVERRIDE.with(|cell| *cell.borrow_mut() = Some(url));
            Self
        }
    }

    impl Drop for OpencodeUrlGuard {
        fn drop(&mut self) {
            OPENCODE_GO_URL_OVERRIDE.with(|cell| *cell.borrow_mut() = None);
        }
    }

    /// Sets the magicube balance endpoint override for the current test thread.
    struct MagicubeUrlGuard;

    impl MagicubeUrlGuard {
        fn set(url: String) -> Self {
            MAGICUBE_URL_OVERRIDE.with(|cell| *cell.borrow_mut() = Some(url));
            Self
        }
    }

    impl Drop for MagicubeUrlGuard {
        fn drop(&mut self) {
            MAGICUBE_URL_OVERRIDE.with(|cell| *cell.borrow_mut() = None);
        }
    }

    /// Records every request path and always answers with the same status/body.
    async fn recording_server(
        status: &'static str,
        body: &'static str,
    ) -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let requests: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = requests.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                let mut bytes = Vec::new();
                let mut buffer = [0_u8; 2048];
                loop {
                    let Ok(read) = socket.read(&mut buffer).await else {
                        break;
                    };
                    if read == 0 {
                        break;
                    }
                    bytes.extend_from_slice(&buffer[..read]);
                    if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }
                let request = String::from_utf8_lossy(&bytes).to_string();
                sink.lock().unwrap().push(request);
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(response.as_bytes()).await;
            }
        });
        (format!("http://{address}"), requests)
    }

    fn opencode_go_site(base_url: &str) -> SiteRow {
        newapi_site(base_url)
    }

    fn opencode_settings() -> AppSettings {
        let mut settings = AppSettings::default();
        settings.proxy_mode = "none".into();
        settings
    }

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 8, 21).unwrap()
    }

    async fn mock_quota_server() -> (String, tokio::task::JoinHandle<Vec<String>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let mut requests = Vec::new();
            for _ in 0..7 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                let mut buffer = [0_u8; 2048];
                loop {
                    let read = socket.read(&mut buffer).await.unwrap();
                    if read == 0 {
                        break;
                    }
                    bytes.extend_from_slice(&buffer[..read]);
                    if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }
                let request = String::from_utf8_lossy(&bytes).to_string();
                let (status, body) = if request.starts_with("GET /api/usage/token ") {
                    (
                        "200 OK",
                        r#"{"code":true,"data":{"total_available":750000,"total_used":250000,"total_granted":1000000,"unlimited_quota":false}}"#,
                    )
                } else if request.starts_with("GET /api/status ") {
                    (
                        "200 OK",
                        r#"{"success":true,"data":{"quota_per_unit":500000,"quota_display_type":"USD"}}"#,
                    )
                } else {
                    ("404 Not Found", "")
                };
                requests.push(request);
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            }
            requests
        });
        (format!("http://{address}"), task)
    }

    fn interpret(grants: Hit, sub: Hit, usage: Hit, token: Hit, fallback: bool) -> RoundOutcome {
        interpret_round(
            &grants, &sub, &usage, &token, "g", "s", "u", "t", 1, 10, fallback,
        )
    }

    #[test]
    fn billing_urls_from_bare_and_v1() {
        let bare = normalize_base_url("https://api.example.com").unwrap();
        let urls = billing_urls(&bare.codex_base_url, today());
        assert_eq!(
            urls.credit_grants,
            "https://api.example.com/v1/dashboard/billing/credit_grants"
        );
        assert_eq!(
            urls.subscription,
            "https://api.example.com/v1/dashboard/billing/subscription"
        );
        assert_eq!(
            urls.usage,
            "https://api.example.com/v1/dashboard/billing/usage?start_date=2026-08-01&end_date=2026-08-22"
        );

        let with_v1 = normalize_base_url("https://api.example.com/v1").unwrap();
        let urls = billing_urls(&with_v1.codex_base_url, today());
        assert_eq!(
            urls.credit_grants,
            "https://api.example.com/v1/dashboard/billing/credit_grants"
        );
        assert_eq!(
            token_usage_url("https://api.example.com"),
            "https://api.example.com/api/usage/token"
        );
        assert_eq!(
            quota_status_url("https://api.example.com"),
            "https://api.example.com/api/status"
        );
        assert_eq!(
            sub2api_usage_url("https://api.example.com/"),
            "https://api.example.com/v1/usage"
        );
    }

    #[tokio::test]
    async fn quota_http_probe_uses_public_fallback_paths_and_correct_auth_headers() {
        let (base, server) = mock_quota_server().await;
        let api_root = format!("{base}/newapi/v1");
        let public_bases = public_api_bases(&api_root);
        let client = reqwest::Client::builder().build().unwrap();

        let round = fetch_round(&client, &api_root, &public_bases, "sk-test", true).await;
        let outcome = finish_round(&round, Instant::now(), 1, "sk-test", false);
        match outcome {
            RoundOutcome::Available(quota) => {
                assert_eq!(quota.source, Some(QuotaSource::TokenUsage));
                assert_eq!(quota.remaining_usd, Some(1.5));
                assert_eq!(quota.used_usd, Some(0.5));
                assert_eq!(quota.total_usd, Some(2.0));
                assert_eq!(quota.unit.as_deref(), Some("USD"));
                assert_eq!(
                    quota.endpoint.as_deref(),
                    Some(format!("{base}/api/usage/token").as_str())
                );
            }
            other => panic!("expected available token quota, got {other:?}"),
        }

        let requests = server.await.unwrap();
        assert_eq!(requests.len(), 7);
        let token_request = requests
            .iter()
            .find(|request| request.starts_with("GET /api/usage/token "))
            .unwrap()
            .to_ascii_lowercase();
        assert!(token_request.contains("authorization: bearer sk-test"));
        let status_request = requests
            .iter()
            .find(|request| request.starts_with("GET /api/status "))
            .unwrap()
            .to_ascii_lowercase();
        assert!(!status_request.contains("authorization:"));
        assert!(requests
            .iter()
            .any(|request| request.starts_with("GET /newapi/api/usage/token ")));
        assert!(requests
            .iter()
            .any(|request| request.starts_with("GET /newapi/api/status ")));
    }

    #[test]
    fn origin_strips_trailing_v1() {
        assert_eq!(
            origin_without_v1("https://api.example.com/v1").as_deref(),
            Some("https://api.example.com")
        );
        assert_eq!(
            origin_without_v1("https://relay.example.com/openai/v1").as_deref(),
            Some("https://relay.example.com/openai")
        );
        assert_eq!(origin_without_v1("https://api.example.com"), None);
        assert_eq!(origin_without_v1("https://v1"), None);
    }

    #[test]
    fn public_api_origin_ignores_openai_compatible_path() {
        assert_eq!(
            site_origin("https://relay.example.com/openai/v1").as_deref(),
            Some("https://relay.example.com")
        );
        assert_eq!(
            site_origin("https://relay.example.com:8443/openai/v1").as_deref(),
            Some("https://relay.example.com:8443")
        );
    }

    #[test]
    fn public_api_candidates_support_path_mounted_and_root_deployments() {
        assert_eq!(
            public_api_bases("https://relay.example.com/newapi/v1"),
            vec![
                "https://relay.example.com/newapi".to_string(),
                "https://relay.example.com".to_string(),
            ]
        );
        assert_eq!(
            public_api_bases("https://relay.example.com/v1"),
            vec!["https://relay.example.com".to_string()]
        );
    }

    #[test]
    fn usage_range_is_month_start_to_tomorrow() {
        let (start, end) = usage_date_range(today());
        assert_eq!(start, "2026-08-01");
        assert_eq!(end, "2026-08-22");
        let (start, end) = usage_date_range(NaiveDate::from_ymd_opt(2026, 1, 31).unwrap());
        assert_eq!(start, "2026-01-01");
        assert_eq!(end, "2026-02-01");
    }

    #[test]
    fn parse_credit_grants_fields() {
        let g = parse_credit_grants(&json!({
            "total_granted": 100.0,
            "total_used": 12.5,
            "total_available": 87.5
        }))
        .unwrap();
        assert_eq!(g.total, Some(100.0));
        assert_eq!(g.used, Some(12.5));
        assert_eq!(g.remaining, Some(87.5));
    }

    #[test]
    fn parse_credit_grants_fills_remaining() {
        let g = parse_credit_grants(&json!({
            "total_granted": "100",
            "total_used": "40"
        }))
        .unwrap();
        assert_eq!(g.remaining, Some(60.0));
    }

    #[test]
    fn parse_credit_grants_rejects_empty_object() {
        assert!(parse_credit_grants(&json!({"object": "credit_summary"})).is_none());
        assert!(parse_credit_grants(&json!([])).is_none());
    }

    #[test]
    fn parse_subscription_prefers_hard_limit() {
        let s = parse_subscription(&json!({
            "hard_limit_usd": 100.0,
            "soft_limit_usd": 80.0,
            "access_until": 1767225600
        }))
        .unwrap();
        assert_eq!(s.limit_usd, Some(100.0));
        assert_eq!(s.expires_at, Some(1767225600));
    }

    #[test]
    fn parse_usage_reads_total_usage() {
        let u = parse_usage(&json!({"object": "list", "total_usage": 2500.0})).unwrap();
        assert_eq!(u.total_usage, 2500.0);
    }

    #[test]
    fn usage_to_usd_divides_by_100() {
        assert_eq!(usage_to_usd(2500.0, Some(100.0)), 25.0);
        assert_eq!(usage_to_usd(2500.0, None), 25.0);
    }

    #[test]
    fn classify_401_and_404() {
        assert_eq!(
            classify_status(401, "{}", Expected::Grants),
            Hit::Unauthorized
        );
        assert_eq!(
            classify_status(404, "nope", Expected::Subscription),
            Hit::NotFound
        );
        assert_eq!(
            classify_status(200, "not-json", Expected::Usage),
            Hit::Unsupported
        );
        assert!(matches!(
            classify_status(502, "down", Expected::Grants),
            Hit::Error(_)
        ));
        assert_eq!(
            classify_status(408, "slow", Expected::Token),
            Hit::Error("request timed out".into())
        );
        assert_eq!(
            classify_status(429, "busy", Expected::Token),
            Hit::Error("HTTP 429".into())
        );
        assert_eq!(
            classify_status(403, "<!DOCTYPE html><html>", Expected::Grants),
            Hit::Unsupported
        );
        assert_eq!(
            classify_status(
                403,
                r#"{"error":{"message":"forbidden"}}"#,
                Expected::Grants
            ),
            Hit::Unauthorized
        );
        assert_eq!(
            classify_status(
                200,
                r#"{"success":false,"message":"token not found"}"#,
                Expected::Token
            ),
            Hit::Unauthorized
        );
        assert_eq!(
            classify_status(
                200,
                r#"{"success":false,"message":"service unavailable"}"#,
                Expected::Token
            ),
            Hit::Error("upstream response indicated failure".into())
        );
        assert_eq!(
            classify_status(
                200,
                r#"{"error":{"message":"database unavailable"}}"#,
                Expected::Grants
            ),
            Hit::Error("upstream response indicated failure".into())
        );
        assert_eq!(
            classify_status(
                200,
                r#"{"success":false,"message":"status unavailable"}"#,
                Expected::Status
            ),
            Hit::Error("upstream response indicated failure".into())
        );
        assert_eq!(
            classify_status(
                200,
                r#"{"code":true,"data":{"total_available":"NaN"}}"#,
                Expected::Token
            ),
            Hit::InvalidData("invalid quota data".into())
        );
    }

    #[test]
    fn parse_token_usage_prefers_display_cny() {
        let parsed = parse_token_usage(&json!({
            "code": true,
            "data": {
                "display": {
                    "remaining": 999.693074,
                    "total": 1000,
                    "unit": "CNY",
                    "used": 0.306926
                },
                "expires_at": 0,
                "total_available": 499846537,
                "total_granted": 500000000,
                "total_used": 153463,
                "unlimited_quota": true
            },
            "message": "ok"
        }))
        .unwrap();
        assert_eq!(parsed.remaining, Some(999.693074));
        assert_eq!(parsed.used, Some(0.306926));
        assert_eq!(parsed.total, Some(1000.0));
        assert_eq!(parsed.unit, "CNY");
        assert!(!parsed.unlimited);
    }

    #[test]
    fn raw_token_usage_respects_unlimited_flag_with_small_negative_balance() {
        let parsed = parse_token_usage(&json!({
            "code": true,
            "data": {
                "total_available": -108_862_302,
                "total_granted": 24_035,
                "total_used": 108_886_337,
                "unlimited_quota": true
            },
            "message": "ok"
        }))
        .unwrap();

        assert_eq!(parsed.remaining, Some(-108_862_302.0));
        assert_eq!(parsed.total, Some(24_035.0));
        assert!(parsed.unlimited);

        let normalized = normalize_token_usage(parsed, None).unwrap();
        assert_eq!(normalized.unit, "RAW_QUOTA");
        let outcome = interpret(
            Hit::NotFound,
            Hit::NotFound,
            Hit::NotFound,
            Hit::Token(normalized),
            false,
        );
        match outcome {
            RoundOutcome::Available(quota) => {
                assert!(quota.unlimited);
                assert_eq!(quota.remaining_usd, None);
                assert_eq!(quota.total_usd, None);
                assert_eq!(quota.unit.as_deref(), Some("RAW_QUOTA"));
            }
            other => panic!("expected unlimited quota, got {other:?}"),
        }
    }

    #[test]
    fn official_new_api_raw_quota_uses_status_metadata_for_usd() {
        let token_body = json!({
            "code": true,
            "data": {
                "total_available": 1_000_000,
                "total_granted": 1_500_000,
                "total_used": 500_000,
                "unlimited_quota": false
            },
            "message": "ok"
        })
        .to_string();
        let status_body = json!({
            "success": true,
            "data": {
                "quota_per_unit": 500_000,
                "quota_display_type": "USD",
                "display_in_currency": true,
                "usd_exchange_rate": 7.2,
                "custom_currency_symbol": "¤",
                "custom_currency_exchange_rate": 2.5
            }
        })
        .to_string();

        let token = classify_status(200, &token_body, Expected::Token);
        let status = classify_status(200, &status_body, Expected::Status);
        let Hit::Token(normalized) = normalize_token_hit(&token, &status) else {
            panic!("expected normalized token usage");
        };
        assert_eq!(normalized.remaining, Some(2.0));
        assert_eq!(normalized.used, Some(1.0));
        assert_eq!(normalized.total, Some(3.0));
        assert_eq!(normalized.unit, "USD");
    }

    #[test]
    fn raw_quota_supports_cny_tokens_and_custom_display_types() {
        let raw = parse_token_usage(&json!({
            "code": true,
            "data": {
                "total_available": 500_000,
                "total_granted": 750_000,
                "total_used": 250_000,
                "unlimited_quota": false
            }
        }))
        .unwrap();
        let cny = parse_quota_status(&json!({
            "success": true,
            "data": {
                "quota_per_unit": 500_000,
                "quota_display_type": "CNY",
                "usd_exchange_rate": 7
            }
        }))
        .unwrap();
        let tokens = parse_quota_status(&json!({
            "success": true,
            "data": {
                "quota_display_type": "TOKENS"
            }
        }))
        .unwrap();
        let custom = parse_quota_status(&json!({
            "success": true,
            "data": {
                "quota_per_unit": 500_000,
                "quota_display_type": "CUSTOM",
                "custom_currency_symbol": "CR",
                "custom_currency_exchange_rate": 2.5
            }
        }))
        .unwrap();

        let cny = normalize_token_usage(raw.clone(), Some(&cny)).unwrap();
        assert_eq!(
            (cny.remaining, cny.used, cny.total),
            (Some(7.0), Some(3.5), Some(10.5))
        );
        assert_eq!(cny.unit, "CNY");

        let tokens = normalize_token_usage(raw.clone(), Some(&tokens)).unwrap();
        assert_eq!(tokens.remaining, Some(500_000.0));
        assert_eq!(tokens.unit, "TOKENS");

        let custom = normalize_token_usage(raw, Some(&custom)).unwrap();
        assert_eq!(custom.remaining, Some(2.5));
        assert_eq!(custom.used, Some(1.25));
        assert!((custom.total.unwrap() - 3.75).abs() < 1e-9);
        assert_eq!(custom.unit, "CR");
    }

    #[test]
    fn legacy_currency_flag_and_missing_status_have_explicit_units() {
        let raw = parse_token_usage(&json!({
            "code": true,
            "data": {
                "total_available": 2,
                "total_granted": 3,
                "total_used": 1,
                "unlimited_quota": false
            }
        }))
        .unwrap();
        let legacy_tokens = parse_quota_status(&json!({
            "success": true,
            "data": {
                "display_in_currency": false
            }
        }))
        .unwrap();
        let no_metadata = parse_quota_status(&json!({
            "success": true,
            "data": {}
        }))
        .unwrap();

        let tokens = normalize_token_usage(raw.clone(), Some(&legacy_tokens)).unwrap();
        assert_eq!(tokens.unit, "TOKENS");
        assert_eq!(no_metadata.display_type, QuotaDisplayType::Raw);
        let raw_from_status = normalize_token_usage(raw.clone(), Some(&no_metadata)).unwrap();
        assert_eq!(raw_from_status.unit, "RAW_QUOTA");
        let raw = normalize_token_usage(raw, None).unwrap();
        assert_eq!(raw.unit, "RAW_QUOTA");
    }

    #[test]
    fn token_display_values_take_priority_over_status_conversion() {
        let token = parse_token_usage(&json!({
            "code": true,
            "data": {
                "display": {
                    "remaining": 7,
                    "used": 3,
                    "total": 10,
                    "unit": "CNY"
                },
                "unlimited_quota": false
            }
        }))
        .unwrap();
        let status = parse_quota_status(&json!({
            "success": true,
            "data": {
                "quota_per_unit": 500_000,
                "quota_display_type": "USD"
            }
        }))
        .unwrap();

        let normalized = normalize_token_usage(token, Some(&status)).unwrap();
        assert_eq!(
            (normalized.remaining, normalized.used, normalized.total),
            (Some(7.0), Some(3.0), Some(10.0))
        );
        assert_eq!(normalized.unit, "CNY");
    }

    #[test]
    fn partial_token_display_does_not_mix_display_and_raw_units() {
        let token = parse_token_usage(&json!({
            "code": true,
            "data": {
                "display": {
                    "remaining": 7,
                    "unit": "CNY"
                },
                "total_available": 500_000,
                "total_granted": 750_000,
                "total_used": 250_000,
                "unlimited_quota": false
            }
        }))
        .unwrap();
        let status = parse_quota_status(&json!({
            "success": true,
            "data": {
                "quota_per_unit": 500_000,
                "quota_display_type": "USD"
            }
        }))
        .unwrap();

        let normalized = normalize_token_usage(token, Some(&status)).unwrap();
        assert_eq!(
            (normalized.remaining, normalized.used, normalized.total),
            (Some(1.0), Some(0.5), Some(1.5))
        );
        assert_eq!(normalized.unit, "USD");
    }

    #[test]
    fn complete_display_values_do_not_use_a_currency_agnostic_unlimited_threshold() {
        let token = parse_token_usage(&json!({
            "code": true,
            "data": {
                "display": {
                    "remaining": 150_000,
                    "used": 50_000,
                    "total": 200_000,
                    "unit": "TOKENS"
                },
                "total_available": -1,
                "total_granted": 1,
                "total_used": 2,
                "unlimited_quota": true
            }
        }))
        .unwrap();

        assert!(!token.unlimited);
        assert_eq!(token.total, Some(200_000.0));
    }

    #[test]
    fn inconsistent_finite_token_data_returns_invalid_data_without_fallback() {
        let token = parse_token_usage(&json!({
            "code": true,
            "data": {
                "total_available": 0,
                "total_granted": 24_035,
                "total_used": 108_886_337,
                "unlimited_quota": false
            },
            "message": "ok"
        }))
        .unwrap();
        let error = normalize_token_usage(token, None).unwrap_err();
        let outcome = interpret(
            Hit::NotFound,
            Hit::NotFound,
            Hit::NotFound,
            Hit::InvalidData(error),
            false,
        );

        match outcome {
            RoundOutcome::Quiet(quota) => {
                assert_eq!(quota.status, QuotaProbeStatus::InvalidData);
                assert_eq!(quota.error.as_deref(), Some("inconsistent quota data"));
            }
            other => panic!("expected invalid data, got {other:?}"),
        }
    }

    #[test]
    fn valid_credit_grants_win_when_token_data_is_invalid() {
        let token = parse_token_usage(&json!({
            "code": true,
            "data": {
                "total_available": 0,
                "total_granted": 10,
                "total_used": 11,
                "unlimited_quota": false
            }
        }))
        .unwrap();
        let token = normalize_token_usage(token, None)
            .map(Hit::Token)
            .unwrap_or_else(Hit::InvalidData);
        let outcome = interpret(
            Hit::Grants(Grants {
                remaining: Some(8.0),
                used: Some(2.0),
                total: Some(10.0),
            }),
            Hit::NotFound,
            Hit::NotFound,
            token,
            false,
        );

        match outcome {
            RoundOutcome::Available(quota) => {
                assert_eq!(quota.source, Some(QuotaSource::CreditGrants));
                assert_eq!(quota.remaining_usd, Some(8.0));
            }
            other => panic!("expected credit grants, got {other:?}"),
        }
    }

    #[test]
    fn invalid_data_status_serializes_with_snake_case_contract() {
        assert_eq!(
            serde_json::to_string(&QuotaProbeStatus::InvalidData).unwrap(),
            r#""invalid_data""#
        );
    }

    #[test]
    fn token_usage_wins_over_dummy_unlimited_subscription() {
        let sub = Hit::Subscription(Subscription {
            limit_usd: Some(100_000_000.0),
            expires_at: None,
        });
        let usage = Hit::Usage(Usage {
            total_usage: 8.9686,
        });
        let token = Hit::Token(TokenUsage {
            remaining: Some(999.69),
            used: Some(0.31),
            total: Some(1000.0),
            unit: "CNY".into(),
            unlimited: false,
            expires_at: None,
            has_display: true,
        });
        let outcome = interpret(Hit::NotFound, sub, usage, token, true);
        match outcome {
            RoundOutcome::Available(q) => {
                assert_eq!(q.source, Some(QuotaSource::TokenUsage));
                assert_eq!(q.remaining_usd, Some(999.69));
                assert_eq!(q.total_usd, Some(1000.0));
                assert_eq!(q.unit.as_deref(), Some("CNY"));
                assert!(!q.unlimited);
            }
            other => panic!("expected available token usage, got {other:?}"),
        }
    }

    #[test]
    fn grants_win_over_subscription() {
        let grants = Hit::Grants(Grants {
            remaining: Some(87.5),
            used: Some(12.5),
            total: Some(100.0),
        });
        let sub = Hit::Subscription(Subscription {
            limit_usd: Some(999.0),
            expires_at: Some(1767225600),
        });
        let usage = Hit::Usage(Usage { total_usage: 1.0 });
        let outcome = interpret_round(
            &grants,
            &sub,
            &usage,
            &Hit::Unsupported,
            "https://api.example.com/v1/dashboard/billing/credit_grants",
            "https://api.example.com/v1/dashboard/billing/subscription",
            "https://api.example.com/v1/dashboard/billing/usage",
            "t",
            1,
            10,
            true,
        );
        match outcome {
            RoundOutcome::Available(q) => {
                assert_eq!(q.source, Some(QuotaSource::CreditGrants));
                assert_eq!(q.remaining_usd, Some(87.5));
                assert_eq!(q.expires_at, Some(1767225600));
                assert_eq!(
                    q.endpoint.as_deref(),
                    Some("https://api.example.com/v1/dashboard/billing/credit_grants")
                );
            }
            other => panic!("expected available, got {other:?}"),
        }
    }

    #[test]
    fn inconsistent_subscription_usage_tries_fallback_before_invalid_data() {
        let grants = Hit::NotFound;
        let sub = Hit::Subscription(Subscription {
            limit_usd: Some(20.0),
            expires_at: None,
        });
        let usage = Hit::Usage(Usage {
            total_usage: 2500.0,
        });
        let outcome = interpret(
            grants.clone(),
            sub.clone(),
            usage.clone(),
            Hit::Unsupported,
            true,
        );
        assert_eq!(outcome, RoundOutcome::Fallback);

        let outcome = interpret(grants, sub, usage, Hit::Unsupported, false);
        match outcome {
            RoundOutcome::Quiet(q) => {
                assert_eq!(q.status, QuotaProbeStatus::InvalidData);
                assert_eq!(q.error.as_deref(), Some("inconsistent quota data"));
            }
            other => panic!("expected invalid data, got {other:?}"),
        }
    }

    #[test]
    fn legacy_hard_limit_is_not_guessed_as_unlimited() {
        let grants = Hit::NotFound;
        let sub = Hit::Subscription(Subscription {
            limit_usd: Some(100_000_000.0),
            expires_at: None,
        });
        let usage = Hit::Unsupported;
        let outcome = interpret(grants, sub, usage, Hit::Unsupported, true);
        match outcome {
            RoundOutcome::Available(q) => {
                assert!(!q.unlimited);
                assert_eq!(q.total_usd, Some(100_000_000.0));
                assert_eq!(q.remaining_usd, None);
                assert_eq!(q.source, Some(QuotaSource::SubscriptionOnly));
            }
            other => panic!("expected available, got {other:?}"),
        }
    }

    #[test]
    fn subscription_expiry_without_amount_is_not_available_quota() {
        let outcome = interpret(
            Hit::NotFound,
            Hit::Subscription(Subscription {
                limit_usd: None,
                expires_at: Some(1_767_225_600),
            }),
            Hit::NotFound,
            Hit::Unsupported,
            false,
        );
        match outcome {
            RoundOutcome::Quiet(quota) => {
                assert_eq!(quota.status, QuotaProbeStatus::Unsupported)
            }
            other => panic!("expected unsupported quota, got {other:?}"),
        }
    }

    #[test]
    fn combined_rounds_keep_strongest_diagnostic_and_source_priority() {
        let unauthorized = interpret(
            Hit::Unauthorized,
            Hit::NotFound,
            Hit::NotFound,
            Hit::Unsupported,
            false,
        );
        let unsupported = interpret(
            Hit::NotFound,
            Hit::NotFound,
            Hit::NotFound,
            Hit::Unsupported,
            false,
        );
        let combined = combine_round_outcomes(unauthorized, unsupported);
        match combined {
            RoundOutcome::Quiet(quota) => {
                assert_eq!(quota.status, QuotaProbeStatus::Unauthorized)
            }
            other => panic!("expected unauthorized quota, got {other:?}"),
        }

        let subscription = interpret(
            Hit::NotFound,
            Hit::Subscription(Subscription {
                limit_usd: Some(100.0),
                expires_at: None,
            }),
            Hit::NotFound,
            Hit::Unsupported,
            false,
        );
        let grants = interpret(
            Hit::Grants(Grants {
                remaining: Some(80.0),
                used: Some(20.0),
                total: Some(100.0),
            }),
            Hit::NotFound,
            Hit::NotFound,
            Hit::Unsupported,
            false,
        );
        let combined = combine_round_outcomes(subscription, grants);
        match combined {
            RoundOutcome::Available(quota) => {
                assert_eq!(quota.source, Some(QuotaSource::CreditGrants))
            }
            other => panic!("expected grants quota, got {other:?}"),
        }
    }

    #[test]
    fn all_404_requests_fallback() {
        let outcome = interpret(
            Hit::NotFound,
            Hit::NotFound,
            Hit::NotFound,
            Hit::NotFound,
            true,
        );
        assert_eq!(outcome, RoundOutcome::Fallback);
        let outcome = interpret(
            Hit::NotFound,
            Hit::NotFound,
            Hit::NotFound,
            Hit::NotFound,
            false,
        );
        match outcome {
            RoundOutcome::Quiet(q) => assert_eq!(q.status, QuotaProbeStatus::Unsupported),
            other => panic!("expected quiet unsupported, got {other:?}"),
        }
    }

    #[test]
    fn unauthorized_falls_back_then_stays_unauthorized() {
        let outcome = interpret(
            Hit::Unauthorized,
            Hit::NotFound,
            Hit::Unsupported,
            Hit::Unsupported,
            true,
        );
        assert_eq!(outcome, RoundOutcome::Fallback);
        let outcome = interpret(
            Hit::Unauthorized,
            Hit::NotFound,
            Hit::Unsupported,
            Hit::Unsupported,
            false,
        );
        match outcome {
            RoundOutcome::Quiet(q) => assert_eq!(q.status, QuotaProbeStatus::Unauthorized),
            other => panic!("expected unauthorized, got {other:?}"),
        }
    }

    #[test]
    fn empty_key_is_unsupported() {
        let q = empty_key_result();
        assert_eq!(q.status, QuotaProbeStatus::Unsupported);
        assert!(q.source.is_none());
    }

    #[test]
    fn redact_replaces_raw_api_key_in_error_hit() {
        let key = "sk-abcdefghijklmnop";
        let hit = Hit::Error(sanitize_error(&format!("proxy failed for {key}"), key));
        match hit {
            Hit::Error(msg) => {
                assert!(!msg.contains(key));
                assert!(msg.contains("sk-a…mnop") || msg.contains("sk-ab"));
            }
            _ => panic!("expected error hit"),
        }
    }

    /// 标准探测全部 404，仅 /api/user/self 放行——验证访问令牌兜底。
    async fn user_self_only_server() -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                let mut buffer = [0_u8; 2048];
                loop {
                    let read = socket.read(&mut buffer).await.unwrap();
                    if read == 0 {
                        break;
                    }
                    bytes.extend_from_slice(&buffer[..read]);
                    if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }
                let request = String::from_utf8_lossy(&bytes).to_string();
                let (status, body) = if request.starts_with("GET /api/user/self ") {
                    (
                        "200 OK",
                        r#"{"success":true,"data":{"quota":1000000,"used_quota":250000,"group":"default"}}"#,
                    )
                } else {
                    ("404 Not Found", "")
                };
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            }
        });
        format!("http://{address}")
    }

    fn newapi_site(base_url: &str) -> SiteRow {
        SiteRow {
            id: "s1".into(),
            name: "R".into(),
            base_url: base_url.into(),
            base_urls: vec![base_url.into()],
            api_key_encrypted: "x".into(),
            key_prefix: "sk-xx".into(),
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
            keys: crate::domain::SiteKeyState {
                active_api_key_id: None,
                api_keys: Vec::new(),
            },
            newapi_access_token_encrypted: Some("encrypted".into()),
            newapi_user_id: Some("42".into()),
            proxy_headers_encrypted: None,
            proxy_header_count: 0,
            zcode_api_type: None,
        }
    }

    #[tokio::test]
    async fn user_self_fallback_reports_account_balance() {
        let base = user_self_only_server().await;
        let site = newapi_site(&base);
        let mut settings = AppSettings::default();
        settings.proxy_mode = "none".into();
        let quota = probe_quota(&site, "sk-test", &settings, Some(("access-token", "42")))
            .await
            .unwrap();
        assert_eq!(quota.status, QuotaProbeStatus::Available);
        assert_eq!(quota.source, Some(QuotaSource::UserSelf));
        assert_eq!(quota.remaining_usd, Some(2.0));
        assert_eq!(quota.used_usd, Some(0.5));
        assert_eq!(quota.total_usd, Some(2.5));
        assert_eq!(quota.unit.as_deref(), Some("USD"));
    }

    #[tokio::test]
    async fn user_self_fallback_is_skipped_without_credentials() {
        let base = user_self_only_server().await;
        let site = newapi_site(&base);
        let mut settings = AppSettings::default();
        settings.proxy_mode = "none".into();
        let quota = probe_quota(&site, "sk-test", &settings, None).await.unwrap();
        assert_eq!(quota.status, QuotaProbeStatus::Unsupported);
    }

    async fn unauthorized_server() -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut buffer = [0_u8; 2048];
                let _ = socket.read(&mut buffer).await;
                let body = r#"{"code":"AUTH_UNAUTHORIZED","message":"Unauthorized, invalid access token","success":false}"#;
                let response = format!(
                    "HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            }
        });
        format!("http://{address}")
    }

    #[tokio::test]
    async fn newapi_access_test_reports_balance_and_rejections() {
        let base = user_self_only_server().await;
        let mut settings = AppSettings::default();
        settings.proxy_mode = "none".into();
        let ok = test_newapi_access(&base, "access-token", "42", &settings)
            .await
            .unwrap();
        assert!(ok.ok);
        assert_eq!(ok.status, 200);
        assert_eq!(ok.remaining_usd, Some(2.0));
        assert_eq!(ok.used_usd, Some(0.5));

        let denied_base = unauthorized_server().await;
        let denied = test_newapi_access(&denied_base, "bad-token", "42", &settings)
            .await
            .unwrap();
        assert!(!denied.ok);
        assert_eq!(denied.status, 401);
        assert!(denied
            .message
            .as_deref()
            .is_some_and(|message| message.contains("invalid access token")));
        // 令牌绝不能出现在返回的消息里
        assert!(!denied
            .message
            .as_deref()
            .is_some_and(|message| message.contains("bad-token")));
    }

    /// 按路径前缀路由响应并记录每条请求；未匹配的路径返回 404。
    async fn routed_server(
        routes: Vec<(&'static str, &'static str, &'static str)>,
    ) -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let requests: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = requests.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                let mut bytes = Vec::new();
                let mut buffer = [0_u8; 2048];
                loop {
                    let Ok(read) = socket.read(&mut buffer).await else {
                        break;
                    };
                    if read == 0 {
                        break;
                    }
                    bytes.extend_from_slice(&buffer[..read]);
                    if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }
                let request = String::from_utf8_lossy(&bytes).to_string();
                sink.lock().unwrap().push(request.clone());
                let (status, body) = routes
                    .iter()
                    .find(|(prefix, _, _)| request.starts_with(prefix))
                    .map(|(_, status, body)| (*status, *body))
                    .unwrap_or(("404 Not Found", ""));
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(response.as_bytes()).await;
            }
        });
        (format!("http://{address}"), requests)
    }

    fn test_settings() -> AppSettings {
        let mut settings = AppSettings::default();
        settings.proxy_mode = "none".into();
        settings
    }

    /// 配了访问令牌时，`/api/user/self` 是最高优先级来源：即便标准链本可返回，
    /// 也应短路，不请求 billing / token 路径。
    #[tokio::test]
    async fn user_self_priority_short_circuits_standard_chain() {
        let (base, requests) = routed_server(vec![
            (
                "GET /api/user/self ",
                "200 OK",
                r#"{"success":true,"data":{"quota":1000000,"used_quota":250000}}"#,
            ),
            (
                "GET /api/usage/token ",
                "200 OK",
                r#"{"code":true,"data":{"total_available":750000,"total_used":250000,"total_granted":1000000,"unlimited_quota":false}}"#,
            ),
            (
                "GET /api/status ",
                "200 OK",
                r#"{"success":true,"data":{"quota_per_unit":500000,"quota_display_type":"USD"}}"#,
            ),
        ])
        .await;
        let site = newapi_site(&base);
        let quota = probe_quota(
            &site,
            "sk-test",
            &test_settings(),
            Some(("access-token", "42")),
        )
        .await
        .unwrap();

        assert_eq!(quota.status, QuotaProbeStatus::Available);
        assert_eq!(quota.source, Some(QuotaSource::UserSelf));
        assert_eq!(quota.remaining_usd, Some(2.0));

        let recorded = requests.lock().unwrap().clone();
        // 只有账户余额请求，及其并发取换算参数的 /api/status。
        assert_eq!(
            recorded.len(),
            2,
            "user self must short-circuit the standard chain"
        );
        assert!(recorded
            .iter()
            .any(|request| request.starts_with("GET /api/user/self ")));
        assert!(recorded
            .iter()
            .any(|request| request.starts_with("GET /api/status ")));
        let request = recorded
            .iter()
            .find(|request| request.starts_with("GET /api/user/self "))
            .unwrap()
            .to_ascii_lowercase();
        assert!(request.contains("authorization: bearer access-token"));
        assert!(request.contains("new-api-user: 42"));
        assert!(!recorded
            .iter()
            .any(|request| request.contains("/dashboard/billing")));
        assert!(!recorded
            .iter()
            .any(|request| request.contains("/api/usage/token")));
    }

    /// 访问令牌链路失败（401）时才回落到标准探测链。
    #[tokio::test]
    async fn user_self_failure_falls_back_to_standard_chain() {
        let (base, requests) = routed_server(vec![
            (
                "GET /api/user/self ",
                "401 Unauthorized",
                r#"{"success":false,"message":"Unauthorized"}"#,
            ),
            (
                "GET /api/usage/token ",
                "200 OK",
                r#"{"code":true,"data":{"total_available":750000,"total_used":250000,"total_granted":1000000,"unlimited_quota":false}}"#,
            ),
            (
                "GET /api/status ",
                "200 OK",
                r#"{"success":true,"data":{"quota_per_unit":500000,"quota_display_type":"USD"}}"#,
            ),
        ])
        .await;
        let site = newapi_site(&base);
        let quota = probe_quota(
            &site,
            "sk-test",
            &test_settings(),
            Some(("bad-token", "42")),
        )
        .await
        .unwrap();

        assert_eq!(quota.status, QuotaProbeStatus::Available);
        assert_eq!(quota.source, Some(QuotaSource::TokenUsage));
        assert_eq!(quota.remaining_usd, Some(1.5));

        let recorded = requests.lock().unwrap().clone();
        assert!(recorded
            .iter()
            .any(|request| request.starts_with("GET /api/user/self ")));
        assert!(recorded
            .iter()
            .any(|request| request.starts_with("GET /api/usage/token ")));
    }

    /// 带路径前缀的 Base URL（`https://host/openai/v1`）只有回退到站点根才能命中
    /// `/api/user/self`。「测试」按钮与主界面额度行必须遍历同一套候选 origin，
    /// 否则两边会显示不同来源的余额（本次修的就是这个不一致）。
    #[tokio::test]
    async fn user_self_candidates_match_between_test_button_and_quota_row() {
        let (base, requests) = routed_server(vec![(
            "GET /api/user/self ",
            "200 OK",
            r#"{"success":true,"data":{"quota":1000000,"used_quota":250000}}"#,
        )])
        .await;
        let base_url = format!("{base}/openai/v1");

        let mut settings = AppSettings::default();
        settings.proxy_mode = "none".into();
        let probed = test_newapi_access(&base_url, "access-token", "42", &settings)
            .await
            .unwrap();
        assert!(probed.ok);
        assert_eq!(probed.remaining_usd, Some(2.0));
        assert_eq!(probed.used_usd, Some(0.5));
        assert_eq!(probed.total_usd, Some(2.5));

        let site = newapi_site(&base_url);
        let quota = probe_quota(&site, "sk-test", &settings, Some(("access-token", "42")))
            .await
            .unwrap();
        assert_eq!(quota.source, Some(QuotaSource::UserSelf));
        assert_eq!(quota.remaining_usd, probed.remaining_usd);
        assert_eq!(quota.used_usd, probed.used_usd);
        assert_eq!(quota.total_usd, probed.total_usd);

        // 两处都必须按同一顺序探测：先路径前缀 origin（404），再回退站点根。
        // 每个候选还会并发取 /api/status 换算参数，这里按路径分别核对顺序。
        let recorded = requests.lock().unwrap().clone();
        let paths: Vec<&str> = recorded
            .iter()
            .filter_map(|request| request.split(' ').nth(1))
            .filter(|path| path.ends_with("/api/user/self"))
            .collect();
        assert_eq!(
            paths,
            vec![
                "/openai/api/user/self",
                "/api/user/self",
                "/openai/api/user/self",
                "/api/user/self",
            ],
            "test button and quota row must probe the same candidate origins in the same order"
        );
        let status_paths: Vec<&str> = recorded
            .iter()
            .filter_map(|request| request.split(' ').nth(1))
            .filter(|path| path.ends_with("/api/status"))
            .collect();
        assert_eq!(
            status_paths,
            vec![
                "/openai/api/status",
                "/api/status",
                "/openai/api/status",
                "/api/status",
            ],
            "the conversion params must be fetched from the same candidate origins"
        );
    }

    /// 非 2xx 但 body 恰好可解析时，两处都必须算失败：否则会出现
    /// 「测试通过、主界面回落到 billing 哨兵值」的分叉。
    #[tokio::test]
    async fn non_2xx_parseable_user_self_body_is_not_a_success_for_either_entrypoint() {
        let (base, _requests) = routed_server(vec![
            (
                "GET /api/user/self ",
                "502 Bad Gateway",
                r#"{"success":true,"data":{"quota":1000000,"used_quota":250000}}"#,
            ),
            (
                "GET /api/usage/token ",
                "200 OK",
                r#"{"code":true,"data":{"total_available":750000,"total_used":250000,"total_granted":1000000,"unlimited_quota":false}}"#,
            ),
            (
                "GET /api/status ",
                "200 OK",
                r#"{"success":true,"data":{"quota_per_unit":500000,"quota_display_type":"USD"}}"#,
            ),
        ])
        .await;

        let mut settings = AppSettings::default();
        settings.proxy_mode = "none".into();
        let probed = test_newapi_access(&base, "access-token", "42", &settings)
            .await
            .unwrap();
        assert!(!probed.ok);
        assert_eq!(probed.status, 502);

        let site = newapi_site(&base);
        let quota = probe_quota(&site, "sk-test", &settings, Some(("access-token", "42")))
            .await
            .unwrap();
        assert_eq!(quota.source, Some(QuotaSource::TokenUsage));
        assert_eq!(quota.remaining_usd, Some(1.5));
    }

    /// 站点自报 `quota_per_unit` 作**除数**、`quota_display_type` 决定货币；
    /// 缺失/非正数回退 500000，缺失 display_type（Raw）回退 USD。
    #[test]
    fn account_balance_scale_uses_self_reported_per_unit_and_currency() {
        let usd = parse_quota_status(&json!({
            "success": true,
            "data": {"quota_per_unit": 100_000, "quota_display_type": "USD"}
        }))
        .unwrap();
        assert_eq!(
            account_balance_scale(Some(&usd)),
            (100_000.0, "USD".to_string())
        );

        let cny = parse_quota_status(&json!({
            "success": true,
            "data": {
                "quota_per_unit": 100_000,
                "quota_display_type": "CNY",
                "usd_exchange_rate": 7.3
            }
        }))
        .unwrap();
        assert_eq!(
            account_balance_scale(Some(&cny)),
            (100_000.0, "CNY".to_string())
        );

        // AgentRouter：无 quota_display_type → Raw，货币保持 USD 现状。
        let raw = parse_quota_status(&json!({
            "success": true,
            "data": {"quota_per_unit": 500_000}
        }))
        .unwrap();
        assert_eq!(raw.display_type, QuotaDisplayType::Raw);
        assert_eq!(
            account_balance_scale(Some(&raw)),
            (500_000.0, "USD".to_string())
        );

        // 缺 quota_per_unit → 除 500000，但货币仍按自报类型。
        let missing = parse_quota_status(&json!({
            "success": true,
            "data": {"quota_display_type": "CNY"}
        }))
        .unwrap();
        assert_eq!(missing.quota_per_unit, None);
        assert_eq!(
            account_balance_scale(Some(&missing)),
            (500_000.0, "CNY".to_string())
        );

        // 非正数在解析阶段就被过滤，除数回退 500000。
        let zero = parse_quota_status(&json!({
            "success": true,
            "data": {"quota_per_unit": 0, "quota_display_type": "USD"}
        }))
        .unwrap();
        assert_eq!(zero.quota_per_unit, None);
        assert_eq!(
            account_balance_scale(Some(&zero)),
            (500_000.0, "USD".to_string())
        );

        // status 完全拿不到 → 默认 500000 / USD。
        assert_eq!(account_balance_scale(None), (500_000.0, "USD".to_string()));
    }

    /// 语义锚点：token 额度是「乘数」，账户余额是「除数」。CNY 下两者数值不同，
    /// 误用乘数会把 10.0 元算成 73.0。
    #[test]
    fn account_balance_scale_is_a_divisor_not_the_token_multiplier() {
        let status = parse_quota_status(&json!({
            "success": true,
            "data": {
                "quota_per_unit": 100_000,
                "quota_display_type": "CNY",
                "usd_exchange_rate": 7.3
            }
        }))
        .unwrap();
        let (scale, unit) = token_quota_scale(&status).unwrap();
        let (divisor, balance_unit) = account_balance_scale(Some(&status));
        assert_eq!(divisor, 100_000.0);
        assert_eq!(balance_unit, "CNY");
        assert_eq!(unit, "CNY");
        assert_eq!(scale, 7.3 / 100_000.0);

        let amounts = user_self_amounts(
            &json!({"quota": 1_000_000, "used_quota": 500_000}),
            Some(&status),
        )
        .unwrap();
        assert_eq!(amounts, (10.0, 5.0, 15.0));
        // 若误用 token 乘数（1_000_000 * 0.000073）会得到 73.0。
        assert_ne!(amounts.0, 1_000_000.0 * scale);
    }

    #[test]
    fn user_self_amounts_fall_back_to_500000_without_status() {
        let amounts = user_self_amounts(
            &json!({"quota": 135_193_229, "used_quota": 0}),
            None,
        )
        .unwrap();
        assert!((amounts.0 - 270.386458).abs() < 1e-9);
        assert_eq!(amounts.1, 0.0);
        assert!((amounts.2 - 270.386458).abs() < 1e-9);
    }

    /// 自报倍率与货币在「测试」按钮与主界面额度行上必须一致（共用链路）。
    #[tokio::test]
    async fn user_self_self_reported_scale_is_shared_by_test_button_and_quota_row() {
        let (base, _requests) = routed_server(vec![
            (
                "GET /api/user/self ",
                "200 OK",
                r#"{"success":true,"data":{"quota":1000000,"used_quota":500000}}"#,
            ),
            (
                "GET /api/status ",
                "200 OK",
                r#"{"success":true,"data":{"quota_per_unit":100000,"quota_display_type":"CNY","usd_exchange_rate":7.3}}"#,
            ),
        ])
        .await;

        let settings = test_settings();
        let probed = test_newapi_access(&base, "access-token", "42", &settings)
            .await
            .unwrap();
        assert!(probed.ok);
        assert_eq!(probed.remaining_usd, Some(10.0));
        assert_eq!(probed.used_usd, Some(5.0));
        assert_eq!(probed.total_usd, Some(15.0));
        // 「测试」按钮必须回传与主界面额度行同源的货币单位（CNY 自报时不再是 $）。
        assert_eq!(probed.unit.as_deref(), Some("CNY"));

        let site = newapi_site(&base);
        let quota = probe_quota(
            &site,
            "sk-test",
            &settings,
            Some(("access-token", "42")),
        )
        .await
        .unwrap();
        assert_eq!(quota.status, QuotaProbeStatus::Available);
        assert_eq!(quota.source, Some(QuotaSource::UserSelf));
        assert_eq!(quota.remaining_usd, probed.remaining_usd);
        assert_eq!(quota.used_usd, probed.used_usd);
        assert_eq!(quota.total_usd, probed.total_usd);
        assert_eq!(quota.unit, probed.unit);
        assert_eq!(quota.unit.as_deref(), Some("CNY"));
    }

    /// `/api/status` 失败绝不能拖垮账户余额：回退 500000/USD 继续显示。
    #[tokio::test]
    async fn user_self_balance_survives_status_failure() {
        let (base, _requests) = routed_server(vec![
            (
                "GET /api/user/self ",
                "200 OK",
                r#"{"success":true,"data":{"quota":135193229,"used_quota":0}}"#,
            ),
            ("GET /api/status ", "500 Internal Server Error", ""),
        ])
        .await;
        let site = newapi_site(&base);
        let quota = probe_quota(
            &site,
            "sk-test",
            &test_settings(),
            Some(("access-token", "42")),
        )
        .await
        .unwrap();
        assert_eq!(quota.status, QuotaProbeStatus::Available);
        assert_eq!(quota.source, Some(QuotaSource::UserSelf));
        assert_eq!(quota.remaining_usd, Some(270.386458));
        assert_eq!(quota.unit.as_deref(), Some("USD"));
    }

    #[test]
    fn parse_sub2api_usage_reads_wallet_balance_without_inventing_used_or_total() {
        let usage = parse_sub2api_usage(&json!({
            "mode": "unrestricted",
            "isValid": true,
            "planName": "钱包余额",
            "balance": 31.31783589,
            "remaining": 31.31783589,
            "unit": "USD",
            "usage": {"daily": 1.0, "total": 2.0},
            "daily_usage": [],
            "model_stats": []
        }))
        .unwrap();
        assert_eq!(usage.remaining, Some(31.31783589));
        assert_eq!(usage.used, None);
        assert_eq!(usage.total, None);
        assert_eq!(usage.unit, "USD");
        assert!(!usage.unlimited);
    }

    #[test]
    fn parse_sub2api_usage_rejects_invalid_or_failed_payloads() {
        assert!(parse_sub2api_usage(&json!({"isValid": false, "balance": 10.0})).is_none());
        assert!(
            parse_sub2api_usage(&json!({"success": false, "message": "token not found"})).is_none()
        );
        assert!(parse_sub2api_usage(&json!({"error": {"message": "unauthorized"}})).is_none());
        assert!(parse_sub2api_usage(&json!({"isValid": true})).is_none());
        assert!(parse_sub2api_usage(&json!([1, 2, 3])).is_none());
    }

    #[test]
    fn parse_sub2api_usage_falls_back_to_balance_and_defaults_unit_to_usd() {
        let usage = parse_sub2api_usage(&json!({"isValid": true, "balance": "12.5"})).unwrap();
        assert_eq!(usage.remaining, Some(12.5));
        assert_eq!(usage.unit, "USD");
        assert_eq!(usage.used, None);
        assert_eq!(usage.total, None);

        let cny = parse_sub2api_usage(&json!({
            "isValid": true,
            "remaining": 3.0,
            "unit": "CNY"
        }))
        .unwrap();
        assert_eq!(cny.unit, "CNY");
    }

    #[test]
    fn parse_sub2api_usage_maps_unrestricted_negative_remaining_to_unlimited() {
        let usage = parse_sub2api_usage(&json!({
            "mode": "unrestricted",
            "isValid": true,
            "balance": -1.0,
            "remaining": -1.0,
            "unit": "USD"
        }))
        .unwrap();
        assert!(usage.unlimited);
        assert_eq!(usage.remaining, None);
        assert_eq!(usage.used, None);
        assert_eq!(usage.total, None);

        // 非 unrestricted 的负值原样保留，交由 clamp_remaining 处理。
        let negative =
            parse_sub2api_usage(&json!({"mode": "quota", "remaining": -1.0})).unwrap();
        assert!(!negative.unlimited);
        assert_eq!(negative.remaining, Some(-1.0));
    }

    /// 端点不可用（404/405/501/HTML/非法 JSON/isValid:false）静默降级为不支持，
    /// 认证失败沿用 Unauthorized 语义。
    #[test]
    fn classify_sub2api_usage_degrades_quietly() {
        assert_eq!(
            classify_status(404, "", Expected::Sub2ApiUsage),
            Hit::NotFound
        );
        assert_eq!(
            classify_status(405, "", Expected::Sub2ApiUsage),
            Hit::NotFound
        );
        assert_eq!(
            classify_status(501, "", Expected::Sub2ApiUsage),
            Hit::NotFound
        );
        assert_eq!(
            classify_status(
                200,
                "<!DOCTYPE html><html><body>welcome</body></html>",
                Expected::Sub2ApiUsage
            ),
            Hit::Unsupported
        );
        assert_eq!(
            classify_status(200, "not-json", Expected::Sub2ApiUsage),
            Hit::Unsupported
        );
        assert_eq!(
            classify_status(200, r#"{"isValid":false,"balance":1.0}"#, Expected::Sub2ApiUsage),
            Hit::Unsupported
        );
        assert_eq!(
            classify_status(403, "<!DOCTYPE html><html>", Expected::Sub2ApiUsage),
            Hit::Unsupported
        );
        assert_eq!(
            classify_status(401, "{}", Expected::Sub2ApiUsage),
            Hit::Unauthorized
        );
        assert_eq!(
            classify_status(
                403,
                r#"{"error":{"message":"forbidden"}}"#,
                Expected::Sub2ApiUsage
            ),
            Hit::Unauthorized
        );
        match classify_status(
            200,
            r#"{"mode":"unrestricted","isValid":true,"balance":31.31,"unit":"USD"}"#,
            Expected::Sub2ApiUsage,
        ) {
            Hit::Sub2ApiUsage(usage) => assert_eq!(usage.remaining, Some(31.31)),
            other => panic!("expected sub2api usage, got {other:?}"),
        }
    }

    /// 标准链命中的 new-api 站点不得因为 Sub2API 探测多打 `/v1/usage`。
    #[tokio::test]
    async fn sub2api_probe_is_skipped_when_standard_chain_hits() {
        let (base, requests) = routed_server(vec![
            (
                "GET /api/usage/token ",
                "200 OK",
                r#"{"code":true,"data":{"total_available":750000,"total_used":250000,"total_granted":1000000,"unlimited_quota":false}}"#,
            ),
            (
                "GET /api/status ",
                "200 OK",
                r#"{"success":true,"data":{"quota_per_unit":500000,"quota_display_type":"USD"}}"#,
            ),
        ])
        .await;
        let site = newapi_site(&base);
        let quota = probe_quota(&site, "sk-test", &test_settings(), None)
            .await
            .unwrap();
        assert_eq!(quota.source, Some(QuotaSource::TokenUsage));
        assert_eq!(quota.remaining_usd, Some(1.5));

        let recorded = requests.lock().unwrap().clone();
        assert!(
            !recorded
                .iter()
                .any(|request| request.contains("/v1/usage")),
            "standard chain hit must not trigger a sub2api request"
        );
    }

    /// Sub2API 站点：标准链全 404 → 命中 `/v1/usage` 钱包余额。
    #[tokio::test]
    async fn sub2api_wallet_balance_is_used_when_standard_chain_is_unsupported() {
        let (base, requests) = routed_server(vec![(
            "GET /v1/usage ",
            "200 OK",
            r#"{"mode":"unrestricted","isValid":true,"planName":"wallet","balance":31.31783589,"remaining":31.31783589,"unit":"USD","usage":{"daily":0,"total":0},"daily_usage":[],"model_stats":[]}"#,
        )])
        .await;
        let site = newapi_site(&base);
        let quota = probe_quota(&site, "sk-test", &test_settings(), None)
            .await
            .unwrap();

        assert_eq!(quota.status, QuotaProbeStatus::Available);
        assert_eq!(quota.source, Some(QuotaSource::Sub2Api));
        assert_eq!(quota.remaining_usd, Some(31.31783589));
        assert_eq!(quota.used_usd, None);
        assert_eq!(quota.total_usd, None);
        assert_eq!(quota.unit.as_deref(), Some("USD"));
        assert!(!quota.unlimited);
        assert_eq!(quota.error, None);
        assert_eq!(
            quota.endpoint.as_deref(),
            Some(format!("{base}/v1/usage").as_str())
        );

        let recorded = requests.lock().unwrap().clone();
        let usage_request = recorded
            .iter()
            .find(|request| request.starts_with("GET /v1/usage "))
            .expect("sub2api endpoint must be requested")
            .to_ascii_lowercase();
        assert!(usage_request.contains("authorization: bearer sk-test"));
        // 现网约束：AiHub 拒绝无 UA 的请求（403），必须带 App UA。
        assert!(
            usage_request.contains("user-agent: xiaobaiswitch"),
            "sub2api probe must send the app user agent, got: {usage_request}"
        );
    }

    /// `/v1/usage` 不可用时安静降级为「不支持」，不出现错误文案。
    #[tokio::test]
    async fn sub2api_endpoint_failures_degrade_to_unsupported() {
        for (status, body) in [
            ("404 Not Found", ""),
            ("405 Method Not Allowed", ""),
            ("501 Not Implemented", ""),
            ("200 OK", "<!DOCTYPE html><html><body>welcome</body></html>"),
            ("200 OK", "not-json"),
            ("200 OK", r#"{"isValid":false,"balance":31.31}"#),
        ] {
            let (base, requests) = routed_server(vec![("GET /v1/usage ", status, body)]).await;
            let site = newapi_site(&base);
            let quota = probe_quota(&site, "sk-test", &test_settings(), None)
                .await
                .unwrap();
            assert_eq!(
                quota.status,
                QuotaProbeStatus::Unsupported,
                "status={status} body={body}"
            );
            assert_eq!(quota.error, None);
            assert_eq!(quota.source, None);
            assert_eq!(quota.remaining_usd, None);
            // 每个降级样例都必须真的打到 /v1/usage，否则这条测试是空转的。
            let recorded = requests.lock().unwrap().clone();
            assert!(
                recorded
                    .iter()
                    .any(|request| request.starts_with("GET /v1/usage ")),
                "status={status} body={body} must still hit the sub2api endpoint"
            );
        }
    }

    /// 候选 origin 复用 `public_api_bases`：路径前缀 origin 不是 Sub2API 时回退站点根。
    #[tokio::test]
    async fn sub2api_usage_falls_back_to_the_site_root_origin() {
        let (base, requests) = routed_server(vec![(
            "GET /v1/usage ",
            "200 OK",
            r#"{"mode":"unrestricted","isValid":true,"balance":31.31783589,"remaining":31.31783589,"unit":"USD"}"#,
        )])
        .await;
        let base_url = format!("{base}/openai/v1");
        let site = newapi_site(&base_url);
        let quota = probe_quota(&site, "sk-test", &test_settings(), None)
            .await
            .unwrap();

        assert_eq!(quota.status, QuotaProbeStatus::Available);
        assert_eq!(quota.source, Some(QuotaSource::Sub2Api));
        assert_eq!(quota.remaining_usd, Some(31.31783589));
        assert_eq!(
            quota.endpoint.as_deref(),
            Some(format!("{base}/v1/usage").as_str())
        );

        let recorded = requests.lock().unwrap().clone();
        let paths: Vec<&str> = recorded
            .iter()
            .filter_map(|request| request.split(' ').nth(1))
            .filter(|path| path.ends_with("/v1/usage"))
            .collect();
        assert_eq!(paths, vec!["/openai/v1/usage", "/v1/usage"]);
    }

    /// 401 是认证失败（不是 UA 拦截）时沿用 Unauthorized 语义。
    #[tokio::test]
    async fn sub2api_unauthorized_surfaces_as_unauthorized() {
        let (base, _requests) = routed_server(vec![(
            "GET /v1/usage ",
            "401 Unauthorized",
            r#"{"error":{"message":"invalid api key"}}"#,
        )])
        .await;
        let site = newapi_site(&base);
        let quota = probe_quota(&site, "sk-bad", &test_settings(), None)
            .await
            .unwrap();
        assert_eq!(quota.status, QuotaProbeStatus::Unauthorized);
        assert_eq!(quota.error, None);
    }

    /// AgentRouter/new-api 的硬编码上限（100000000）是「无限额度」哨兵值：
    /// 不能按「总额 - 已用」换算成荒谬的美元余额。
    #[tokio::test]
    async fn absurd_hard_limit_is_reported_as_unlimited_not_usd() {
        let (base, _requests) = routed_server(vec![
            (
                "GET /dashboard/billing/subscription",
                "200 OK",
                r#"{"hard_limit_usd":100000000,"soft_limit_usd":100000000}"#,
            ),
            (
                "GET /dashboard/billing/usage",
                "200 OK",
                r#"{"total_usage":30363}"#,
            ),
        ])
        .await;
        let site = newapi_site(&base);
        let quota = probe_quota(&site, "sk-test", &test_settings(), None)
            .await
            .unwrap();

        assert_eq!(quota.status, QuotaProbeStatus::Available);
        assert_eq!(quota.error, None);
        assert!(quota.unlimited);
        assert_eq!(quota.remaining_usd, None);
        assert_eq!(quota.used_usd, None);
        assert_eq!(quota.total_usd, None);
    }

    #[test]
    fn sanitize_absurd_amounts_flags_sentinel_and_clears_amounts() {
        let mut quota = quiet(QuotaProbeStatus::Available, None, 1, None);
        quota.remaining_usd = Some(99_999_696.37);
        quota.used_usd = Some(303.63);
        quota.total_usd = Some(100_000_000.0);
        quota.source = Some(QuotaSource::SubscriptionUsage);

        let sanitized = sanitize_absurd_amounts(quota);
        assert!(sanitized.unlimited);
        assert_eq!(sanitized.remaining_usd, None);
        assert_eq!(sanitized.used_usd, None);
        assert_eq!(sanitized.total_usd, None);
        assert_eq!(sanitized.status, QuotaProbeStatus::Available);
        assert_eq!(sanitized.error, None);
        assert_eq!(sanitized.source, Some(QuotaSource::SubscriptionUsage));
    }

    #[test]
    fn sanitize_absurd_amounts_keeps_real_balances() {
        let mut quota = quiet(QuotaProbeStatus::Available, None, 1, None);
        quota.remaining_usd = Some(1223.89);
        quota.used_usd = Some(303.63);
        quota.total_usd = Some(1527.52);
        quota.source = Some(QuotaSource::UserSelf);

        assert_eq!(sanitize_absurd_amounts(quota.clone()), quota);
    }

    /// 只有「剩余金额」能触发哨兵判定：某站的累计已用 / 总额天然巨大时，
    /// 不能连真实可用的剩余金额一起清掉、误报成「不限额」。
    #[test]
    fn sanitize_absurd_amounts_keeps_real_remaining_when_totals_look_absurd() {
        let mut quota = quiet(QuotaProbeStatus::Available, None, 1, None);
        quota.remaining_usd = Some(500.0);
        quota.used_usd = Some(2_000_000.0);
        quota.total_usd = Some(2_000_500.0);
        quota.source = Some(QuotaSource::SubscriptionUsage);

        let sanitized = sanitize_absurd_amounts(quota.clone());
        assert!(!sanitized.unlimited);
        assert_eq!(sanitized.remaining_usd, Some(500.0));
        assert_eq!(sanitized, quota);
    }

    /// 阈值以下的真实大额余额照常展示，不猜成「不限额」。
    #[test]
    fn sanitize_absurd_amounts_keeps_large_but_plausible_balance() {
        let mut quota = quiet(QuotaProbeStatus::Available, None, 1, None);
        quota.remaining_usd = Some(2_000_000.0);
        quota.used_usd = Some(10.0);
        quota.total_usd = Some(2_000_010.0);
        quota.source = Some(QuotaSource::UserSelf);

        assert_eq!(sanitize_absurd_amounts(quota.clone()), quota);
    }

    #[test]
    fn opencode_go_base_detection() {
        assert!(is_opencode_go_base("https://opencode.ai/zen/go/v1"));
        assert!(is_opencode_go_base("https://opencode.ai/zen/go"));
        assert!(is_opencode_go_base("https://opencode.ai/zen/go/v1/"));
        assert!(is_opencode_go_base("https://opencode.ai/zen/go?x=1"));
        assert!(!is_opencode_go_base("https://opencode.ai/zen/v1"));
        assert!(!is_opencode_go_base("https://opencode.ai"));
        assert!(!is_opencode_go_base("https://api.opencode.ai/zen/go/v1"));
        assert!(!is_opencode_go_base("https://opencode.ai.evil.com/zen/go/v1"));
        assert!(!is_opencode_go_base("http://opencode.ai/zen/go/v1"));
        assert!(!is_opencode_go_base("ftp://opencode.ai/zen/go/v1"));
        assert!(!is_opencode_go_base("not a url"));
        // `/zen/go` 必须按 path segment 边界匹配
        assert!(!is_opencode_go_base("https://opencode.ai/zen/gopher"));
        assert!(!is_opencode_go_base("https://opencode.ai/zen/go-v2/v1"));
        assert!(!is_opencode_go_base("https://opencode.ai/zen/goose/v1"));
    }

    #[test]
    fn opencode_go_usage_url_is_fixed_official_path() {
        assert_eq!(
            opencode_go_usage_url("https://opencode.ai/zen/go/v1").as_deref(),
            Some("https://opencode.ai/zen/go/v1/usage")
        );
        // 固定路径：任何 opencode.ai origin 都拼到官方 usage 端点
        assert_eq!(
            opencode_go_usage_url("https://opencode.ai/zen/go").as_deref(),
            Some("https://opencode.ai/zen/go/v1/usage")
        );
        assert_eq!(opencode_go_usage_url("https://api.opencode.ai/zen/go/v1"), None);
        assert_eq!(opencode_go_usage_url("https://example.com/zen/go/v1"), None);
        assert_eq!(opencode_go_usage_url("http://opencode.ai/zen/go/v1"), None);
    }

    #[test]
    fn opencode_go_parses_camel_case_windows_and_relative_reset() {
        let fetched_at = 1_767_000_000_000_i64;
        let windows = parse_opencode_go_windows(
            &json!({
                "rollingUsage": {"usagePercent": 12.5, "resetInSec": 3600, "limit": 12},
                "weeklyUsage": {"usagePercent": 46.2, "resetInSec": 86400, "limitUsd": 30},
                "monthlyUsage": {"usagePercent": 8.4, "resetAt": 1_767_225_600_i64, "limit_usd": 60}
            }),
            fetched_at,
        )
        .unwrap();

        assert_eq!(windows.len(), 3);
        assert_eq!(windows[0].kind, "rolling");
        assert_eq!(windows[0].usage_percent, Some(12.5));
        assert_eq!(windows[0].reset_at, Some(fetched_at + 3_600_000));
        assert_eq!(windows[0].limit_usd, Some(12.0));
        assert_eq!(windows[1].kind, "weekly");
        assert_eq!(windows[1].reset_at, Some(fetched_at + 86_400_000));
        assert_eq!(windows[1].limit_usd, Some(30.0));
        assert_eq!(windows[2].kind, "monthly");
        // 绝对秒级时间戳被换算为毫秒
        assert_eq!(windows[2].reset_at, Some(1_767_225_600_000));
        assert_eq!(windows[2].limit_usd, Some(60.0));
    }

    #[test]
    fn opencode_go_parses_snake_case_nested_windows() {
        let fetched_at = 1_767_000_000_000_i64;
        let windows = parse_opencode_go_windows(
            &json!({
                "windows": {
                    "rolling": {"usage_percent": 10.0, "resets_in_seconds": 7200, "limit_usd": 12},
                    "weekly": {"percent": 20.0, "reset_in_seconds": 3600},
                    "monthly": {"usage_percent": 30.0, "reset_at": 1_767_225_600_i64}
                }
            }),
            fetched_at,
        )
        .unwrap();

        assert_eq!(windows.len(), 3);
        assert_eq!(windows[0].kind, "rolling");
        assert_eq!(windows[0].usage_percent, Some(10.0));
        assert_eq!(windows[0].reset_at, Some(fetched_at + 7_200_000));
        assert_eq!(windows[1].kind, "weekly");
        assert_eq!(windows[1].usage_percent, Some(20.0));
        assert_eq!(windows[1].reset_at, Some(fetched_at + 3_600_000));
        assert_eq!(windows[2].usage_percent, Some(30.0));
        assert_eq!(windows[2].reset_at, Some(1_767_225_600_000));
    }

    #[test]
    fn opencode_go_parses_production_usage_shape_with_resets_at() {
        // 精确复刻 formatUsage 产出：usage.rolling|weekly|monthly + percent + resetsAt(RFC3339)。
        let windows = parse_opencode_go_windows(
            &json!({
                "usage": {
                    "rolling": {
                        "status": "ok",
                        "percent": 12,
                        "resetsAt": "2026-09-13T06:06:01.287Z"
                    },
                    "weekly": {
                        "status": "ok",
                        "percent": 30,
                        "resetsAt": "2026-09-14T06:06:01.287Z"
                    },
                    "monthly": {
                        "status": "ok",
                        "percent": 55,
                        "resetsAt": "2026-10-01T00:00:00.000Z"
                    }
                }
            }),
            1_700_000_000_000,
        )
        .unwrap();

        assert_eq!(windows.len(), 3);
        assert_eq!(windows[0].kind, "rolling");
        assert_eq!(windows[0].usage_percent, Some(12.0));
        assert_eq!(windows[0].reset_at, Some(1_789_279_561_287));
        assert_eq!(windows[1].kind, "weekly");
        assert_eq!(windows[1].usage_percent, Some(30.0));
        assert_eq!(windows[1].reset_at, Some(1_789_365_961_287));
        assert_eq!(windows[2].kind, "monthly");
        assert_eq!(windows[2].usage_percent, Some(55.0));
        assert_eq!(windows[2].reset_at, Some(1_790_812_800_000));
    }

    #[test]
    fn opencode_go_unparseable_resets_at_is_ignored_not_zero() {
        let windows = parse_opencode_go_windows(
            &json!({
                "usage": {
                    "rolling": {"percent": 5, "resetsAt": "not-a-timestamp"},
                    "weekly": {"percent": 6, "resets_at": ""},
                    "monthly": {"percent": 7, "resetsAt": "2026-09-13T06:06:01.287Z"}
                }
            }),
            1_700_000_000_000,
        )
        .unwrap();

        assert_eq!(windows.len(), 3);
        assert_eq!(windows[0].reset_at, None);
        assert_eq!(windows[1].reset_at, None);
        assert_eq!(windows[2].reset_at, Some(1_789_279_561_287));
    }

    #[test]
    fn opencode_go_status_branches() {
        let unauthorized = opencode_go_quota(401, "{}", 1, 5, "https://opencode.ai/zen/go/v1/usage");
        assert_eq!(unauthorized.status, QuotaProbeStatus::Unauthorized);
        assert!(unauthorized.windows.is_empty());

        let not_found = opencode_go_quota(404, "", 1, 5, "https://opencode.ai/zen/go/v1/usage");
        assert_eq!(not_found.status, QuotaProbeStatus::Unsupported);

        let server_error = opencode_go_quota(502, "down", 1, 5, "u");
        assert_eq!(server_error.status, QuotaProbeStatus::Error);
        assert_eq!(server_error.error.as_deref(), Some("HTTP 502"));
    }

    #[test]
    fn opencode_go_forbidden_is_unsupported_not_unauthorized() {
        // 403 = key 有效但无 Go 订阅（EntitlementError），不能提示「换 key」。
        let forbidden = opencode_go_quota(403, "{}", 1, 5, "https://opencode.ai/zen/go/v1/usage");
        assert_eq!(forbidden.status, QuotaProbeStatus::Unsupported);
        assert_ne!(forbidden.status, QuotaProbeStatus::Unauthorized);
    }

    #[tokio::test]
    async fn opencode_go_probe_returns_three_windows_from_mock_server() {
        let body = r#"{"rollingUsage":{"usagePercent":12.5,"resetInSec":3600,"limit":12},"weeklyUsage":{"usagePercent":46.2,"resetInSec":86400,"limitUsd":30},"monthlyUsage":{"usagePercent":8.4,"resetAt":1767225600,"limit_usd":60}}"#;
        let (base, _requests) = recording_server("200 OK", body).await;
        let _guard = OpencodeUrlGuard::set(format!("{base}/zen/go/v1/usage"));
        let site = opencode_go_site("https://opencode.ai/zen/go/v1");
        let before = Utc::now().timestamp_millis();
        let quota = probe_quota(&site, "sk-opencode", &opencode_settings(), None)
            .await
            .unwrap();

        assert_eq!(quota.status, QuotaProbeStatus::Available);
        assert_eq!(quota.source, Some(QuotaSource::OpencodeGo));
        assert_eq!(quota.unit.as_deref(), Some("USD"));
        assert_eq!(quota.windows.len(), 3);
        assert_eq!(quota.windows[0].kind, "rolling");
        assert_eq!(quota.windows[0].usage_percent, Some(12.5));
        let rolling_reset = quota.windows[0].reset_at.unwrap();
        assert!(
            (rolling_reset - (before + 3_600_000)).abs() < 10_000,
            "relative reset seconds should become an absolute timestamp"
        );
        assert_eq!(quota.windows[2].reset_at, Some(1_767_225_600_000));
        assert!(quota
            .endpoint
            .as_deref()
            .is_some_and(|endpoint| endpoint.ends_with("/zen/go/v1/usage")));
    }

    #[tokio::test]
    async fn opencode_go_probe_reports_unauthorized_and_unsupported() {
        let (base, _requests) = recording_server("401 Unauthorized", "{}").await;
        let _guard = OpencodeUrlGuard::set(format!("{base}/zen/go/v1/usage"));
        let site = opencode_go_site("https://opencode.ai/zen/go/v1");
        let quota = probe_quota(&site, "sk-bad", &opencode_settings(), None)
            .await
            .unwrap();
        assert_eq!(quota.status, QuotaProbeStatus::Unauthorized);
        drop(_guard);

        let (base, _requests) = recording_server("404 Not Found", "").await;
        let _guard = OpencodeUrlGuard::set(format!("{base}/zen/go/v1/usage"));
        let quota = probe_quota(&site, "sk-bad", &opencode_settings(), None)
            .await
            .unwrap();
        assert_eq!(quota.status, QuotaProbeStatus::Unsupported);
    }

    #[tokio::test]
    async fn opencode_go_route_skips_billing_probe_paths() {
        let body = r#"{"rollingUsage":{"usagePercent":1,"resetInSec":60},"weeklyUsage":{"usagePercent":2,"resetInSec":120},"monthlyUsage":{"usagePercent":3,"resetInSec":180}}"#;
        let (base, requests) = recording_server("200 OK", body).await;
        let _guard = OpencodeUrlGuard::set(format!("{base}/zen/go/v1/usage"));
        let site = opencode_go_site("https://opencode.ai/zen/go/v1");
        let quota = probe_quota(&site, "sk-opencode", &opencode_settings(), Some(("token", "42")))
            .await
            .unwrap();

        assert_eq!(quota.source, Some(QuotaSource::OpencodeGo));
        let recorded = requests.lock().unwrap().clone();
        assert_eq!(recorded.len(), 1, "only the usage endpoint should be requested");
        assert!(recorded[0].starts_with("GET /zen/go/v1/usage "));
        assert!(recorded[0].to_ascii_lowercase().contains("authorization: bearer sk-opencode"));
        assert!(!recorded
            .iter()
            .any(|request| request.contains("/dashboard/billing")));
        assert!(!recorded
            .iter()
            .any(|request| request.contains("/api/usage/token")));
    }

    #[tokio::test]
    async fn opencode_go_empty_key_is_unauthorized_without_request() {
        let (base, requests) = recording_server("200 OK", "{}").await;
        let _guard = OpencodeUrlGuard::set(format!("{base}/zen/go/v1/usage"));
        let site = opencode_go_site("https://opencode.ai/zen/go/v1");
        let quota = probe_quota(&site, "  ", &opencode_settings(), None)
            .await
            .unwrap();
        assert_eq!(quota.status, QuotaProbeStatus::Unauthorized);
        assert!(requests.lock().unwrap().is_empty());
    }

    fn modelscope_site() -> SiteRow {
        newapi_site("https://api-inference.modelscope.cn/v1")
    }

    #[test]
    fn modelscope_base_detection() {
        assert!(is_modelscope_base("https://api-inference.modelscope.cn/v1"));
        assert!(is_modelscope_base("https://api-inference.modelscope.cn"));
        assert!(is_modelscope_base("https://api-inference.modelscope.cn/v1/"));
        assert!(is_modelscope_base("https://api-inference.modelscope.cn/v1?x=1"));
        // 只认推理 host；站点主站与形似 host 都不算
        assert!(!is_modelscope_base("https://modelscope.cn/v1"));
        assert!(!is_modelscope_base("https://www.modelscope.cn/v1"));
        assert!(!is_modelscope_base("https://api-inference.modelscope.cn.evil.com/v1"));
        assert!(!is_modelscope_base("https://evil.com/api-inference.modelscope.cn/v1"));
        assert!(!is_modelscope_base("http://api-inference.modelscope.cn/v1"));
        assert!(!is_modelscope_base("ftp://api-inference.modelscope.cn/v1"));
        assert!(!is_modelscope_base("not a url"));
    }

    #[test]
    fn magicube_balance_url_is_a_constant_not_derived_from_base_url() {
        // 余额端点与推理端点不同域，必须写死；从 base_url 派生等于把用户可控 host
        // 拼进带 Bearer 的请求。
        assert_eq!(
            magicube_balance_url(),
            "https://modelscope.cn/openapi/v1/magicubes/balance"
        );
        assert_eq!(magicube_balance_url(), MAGICUBE_BALANCE_URL);
    }

    #[test]
    fn magicube_quota_available_maps_balances_onto_point_fields() {
        let quota = magicube_quota(
            200,
            r#"{"success":true,"request_id":"r1","data":{"total_balance":1200.75,"available_balance":1000.5,"frozen_amount":200.25}}"#,
            1_767_000_000_000,
            7,
            MAGICUBE_BALANCE_URL,
        );
        assert_eq!(quota.status, QuotaProbeStatus::Available);
        assert_eq!(quota.source, Some(QuotaSource::MagicubeBalance));
        assert_eq!(quota.unit.as_deref(), Some("MAGICUBE"));
        assert_eq!(quota.remaining_usd, Some(1000.5));
        assert_eq!(quota.total_usd, Some(1200.75));
        // 冻结额与「已花费」无关：不显示，也不用它凑数。
        assert_eq!(quota.used_usd, None);
        assert!(!quota.unlimited);
        assert_eq!(quota.expires_at, None);
        assert_eq!(quota.windows, Vec::new());
        assert_eq!(quota.endpoint.as_deref(), Some(MAGICUBE_BALANCE_URL));
    }

    #[test]
    fn magicube_quota_accepts_available_balance_without_total() {
        let quota = magicube_quota(
            200,
            r#"{"success":true,"data":{"available_balance":42}}"#,
            1,
            2,
            "u",
        );
        assert_eq!(quota.status, QuotaProbeStatus::Available);
        assert_eq!(quota.remaining_usd, Some(42.0));
        assert_eq!(quota.total_usd, None);
    }

    /// 状态映射表：只有「200 + 有限数值 available_balance」才是成功，其余一律不兜底成 0。
    #[test]
    fn magicube_quota_mapping_table() {
        let cases: &[(&str, &str, QuotaProbeStatus)] = &[
            ("missing data", r#"{"success":true}"#, QuotaProbeStatus::InvalidData),
            (
                "null data",
                r#"{"success":true,"data":null}"#,
                QuotaProbeStatus::InvalidData,
            ),
            (
                "missing balance",
                r#"{"success":true,"data":{"total_balance":10}}"#,
                QuotaProbeStatus::InvalidData,
            ),
            (
                "non numeric balance",
                r#"{"success":true,"data":{"available_balance":"abc"}}"#,
                QuotaProbeStatus::InvalidData,
            ),
            (
                "nan balance",
                r#"{"success":true,"data":{"available_balance":"NaN"}}"#,
                QuotaProbeStatus::InvalidData,
            ),
            (
                "infinity balance",
                r#"{"success":true,"data":{"available_balance":"Infinity"}}"#,
                QuotaProbeStatus::InvalidData,
            ),
            (
                "object balance",
                r#"{"success":true,"data":{"available_balance":{}}}"#,
                QuotaProbeStatus::InvalidData,
            ),
            (
                "negative balance",
                r#"{"success":true,"data":{"available_balance":-5}}"#,
                QuotaProbeStatus::InvalidData,
            ),
            (
                "inconsistent totals",
                r#"{"success":true,"data":{"available_balance":20,"total_balance":10}}"#,
                QuotaProbeStatus::InvalidData,
            ),
            (
                "success false",
                r#"{"success":false,"code":"Throttling","message":"no"}"#,
                QuotaProbeStatus::InvalidData,
            ),
            ("html body", "<html><body>hi</body></html>", QuotaProbeStatus::InvalidData),
            ("broken json", "{", QuotaProbeStatus::InvalidData),
            ("unauthorized 401", "{}", QuotaProbeStatus::Unauthorized),
            ("forbidden 403", "{}", QuotaProbeStatus::Unauthorized),
            ("not found 404", "{}", QuotaProbeStatus::Error),
            ("server error 500", "{}", QuotaProbeStatus::Error),
            ("bad gateway 502", "down", QuotaProbeStatus::Error),
        ];
        for (name, body, expected) in cases {
            let status: u16 = match *name {
                "unauthorized 401" => 401,
                "forbidden 403" => 403,
                "not found 404" => 404,
                "server error 500" => 500,
                "bad gateway 502" => 502,
                _ => 200,
            };
            let quota = magicube_quota(status, body, 1, 2, "u");
            assert_eq!(&quota.status, expected, "case `{name}` (HTTP {status})");
            assert_eq!(quota.remaining_usd, None, "case `{name}` must not fabricate a balance");
            assert_eq!(quota.total_usd, None, "case `{name}`");
            assert_eq!(quota.source, None, "case `{name}`");
            assert!(!quota.unlimited, "case `{name}`");
        }
    }

    #[test]
    fn magicube_quota_error_text_carries_no_token_or_upstream_message() {
        // 只有状态码与 code 这类稳定标识可以外流；上游自由文本可能回显请求头。
        let leaked = magicube_quota(
            500,
            r#"{"success":false,"code":"Internal","message":"Bearer sk-super-secret rejected"}"#,
            1,
            2,
            "u",
        );
        assert_eq!(leaked.error.as_deref(), Some("HTTP 500"));

        let rejected = magicube_quota(
            200,
            r#"{"success":false,"code":"InvalidToken","message":"Bearer sk-super-secret rejected"}"#,
            1,
            2,
            "u",
        );
        assert_eq!(rejected.error.as_deref(), Some("code=InvalidToken"));

        let codeless = magicube_quota(200, r#"{"success":false}"#, 1, 2, "u");
        assert_eq!(codeless.error.as_deref(), Some("success=false"));

        for quota in [leaked, rejected, codeless] {
            assert!(
                !quota
                    .error
                    .unwrap_or_default()
                    .contains("sk-super-secret"),
                "error text must never echo the API key"
            );
        }
    }

    #[test]
    fn magicube_quota_truncates_a_runaway_code_field() {
        let code = "x".repeat(400);
        let quota = magicube_quota(200, &format!(r#"{{"success":false,"code":"{code}"}}"#), 1, 2, "u");
        let error = quota.error.unwrap();
        assert!(error.starts_with("code="));
        assert!(error.chars().count() <= 64 + "code=".chars().count());
        assert!(!error.contains('\n'));
    }

    #[test]
    fn allows_empty_key_probe_covers_both_dedicated_hosts() {
        assert!(allows_empty_key_probe("https://opencode.ai/zen/go/v1"));
        assert!(allows_empty_key_probe("https://api-inference.modelscope.cn/v1"));
        // 其余站点仍在命令层短路，不给通用链发空 key 请求。
        assert!(!allows_empty_key_probe("https://api.opencode.ai/zen/go/v1"));
        assert!(!allows_empty_key_probe("https://relay.example.com/v1"));
        assert!(!allows_empty_key_probe("not a url"));
    }

    #[tokio::test]
    async fn magicube_probe_returns_balance_from_mock_server() {
        let body = r#"{"success":true,"request_id":"r1","data":{"total_balance":1200.75,"available_balance":1000.5,"frozen_amount":200.25}}"#;
        let (base, _requests) = recording_server("200 OK", body).await;
        let _guard = MagicubeUrlGuard::set(format!("{base}/openapi/v1/magicubes/balance"));
        let before = Utc::now().timestamp_millis();
        let quota = probe_quota(&modelscope_site(), "sk-modelscope", &opencode_settings(), None)
            .await
            .unwrap();

        assert_eq!(quota.status, QuotaProbeStatus::Available);
        assert_eq!(quota.source, Some(QuotaSource::MagicubeBalance));
        assert_eq!(quota.unit.as_deref(), Some("MAGICUBE"));
        assert_eq!(quota.remaining_usd, Some(1000.5));
        assert_eq!(quota.total_usd, Some(1200.75));
        assert!(quota.fetched_at >= before);
    }

    #[tokio::test]
    async fn magicube_route_skips_billing_and_newapi_probe_paths() {
        let body = r#"{"success":true,"data":{"available_balance":42}}"#;
        let (base, requests) = recording_server("200 OK", body).await;
        let _guard = MagicubeUrlGuard::set(format!("{base}/openapi/v1/magicubes/balance"));
        // 传入 newapi 凭据也必须被忽略：魔搭只用站点自己的 API Key。
        let quota = probe_quota(
            &modelscope_site(),
            "sk-modelscope",
            &opencode_settings(),
            Some(("access-token", "42")),
        )
        .await
        .unwrap();

        assert_eq!(quota.source, Some(QuotaSource::MagicubeBalance));
        let recorded = requests.lock().unwrap().clone();
        assert_eq!(recorded.len(), 1, "only the balance endpoint should be requested");
        assert!(recorded[0].starts_with("GET /openapi/v1/magicubes/balance "));
        assert!(recorded[0]
            .to_ascii_lowercase()
            .contains("authorization: bearer sk-modelscope"));
        assert!(!recorded.iter().any(|r| r.contains("/dashboard/billing")));
        assert!(!recorded.iter().any(|r| r.contains("/api/usage/token")));
        assert!(!recorded.iter().any(|r| r.contains("/api/user/self")));
    }

    #[tokio::test]
    async fn magicube_probe_maps_http_status_without_fabricating_balance() {
        for (status, expected) in [
            ("401 Unauthorized", QuotaProbeStatus::Unauthorized),
            ("403 Forbidden", QuotaProbeStatus::Unauthorized),
            ("500 Internal Server Error", QuotaProbeStatus::Error),
            ("200 OK", QuotaProbeStatus::InvalidData),
        ] {
            let (base, _requests) = recording_server(status, r#"{"success":true,"data":{}}"#).await;
            let _guard = MagicubeUrlGuard::set(format!("{base}/openapi/v1/magicubes/balance"));
            let quota = probe_quota(&modelscope_site(), "sk-x", &opencode_settings(), None)
                .await
                .unwrap();
            assert_eq!(quota.status, expected, "upstream {status}");
            assert_eq!(quota.remaining_usd, None, "upstream {status}");
        }
    }

    #[tokio::test]
    async fn magicube_probe_transport_failure_is_error_without_token() {
        // 端口探测前先释放监听，得到确定性的「连不上」而不是 8 秒超时。
        let (base, listener) = {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            (base, listener)
        };
        drop(listener);
        let _guard = MagicubeUrlGuard::set(format!("{base}/openapi/v1/magicubes/balance"));
        let quota = probe_quota(&modelscope_site(), "sk-super-secret", &opencode_settings(), None)
            .await
            .unwrap();
        assert_eq!(quota.status, QuotaProbeStatus::Error);
        let error = quota.error.unwrap_or_default();
        assert!(!error.contains("sk-super-secret"), "error leaked the key: {error}");
    }

    #[tokio::test]
    async fn magicube_empty_key_is_unauthorized_without_request() {
        let (base, requests) = recording_server("200 OK", "{}").await;
        let _guard = MagicubeUrlGuard::set(format!("{base}/openapi/v1/magicubes/balance"));
        let quota = probe_quota(&modelscope_site(), "  ", &opencode_settings(), None)
            .await
            .unwrap();
        assert_eq!(quota.status, QuotaProbeStatus::Unauthorized);
        assert_eq!(quota.remaining_usd, None);
        assert!(requests.lock().unwrap().is_empty());
    }
}
