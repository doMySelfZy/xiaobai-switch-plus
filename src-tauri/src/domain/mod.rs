use crate::capabilities::SiteCapabilities;
use crate::crypto::key_fingerprint;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

mod thinking;
pub use thinking::*;
mod mcp;
pub use mcp::*;
mod proxy;
pub use proxy::*;
mod rules;
pub use rules::*;
mod agent_update;
pub use agent_update::*;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SiteProtocol {
    OpenaiCompatible,
    Anthropic,
}

impl SiteProtocol {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::OpenaiCompatible => "openai_compatible",
            Self::Anthropic => "anthropic",
        }
    }
    pub fn parse(s: &str) -> Self {
        match s {
            "anthropic" => Self::Anthropic,
            _ => Self::OpenaiCompatible,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ClaudeAuthKeyStyle {
    AnthropicAuthToken,
    AnthropicApiKey,
}

impl ClaudeAuthKeyStyle {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AnthropicAuthToken => "anthropic_auth_token",
            Self::AnthropicApiKey => "anthropic_api_key",
        }
    }
    pub fn parse(s: &str) -> Self {
        match s {
            "anthropic_api_key" => Self::AnthropicApiKey,
            _ => Self::AnthropicAuthToken,
        }
    }
    pub fn env_key(&self) -> &'static str {
        match self {
            Self::AnthropicAuthToken => "ANTHROPIC_AUTH_TOKEN",
            Self::AnthropicApiKey => "ANTHROPIC_API_KEY",
        }
    }
    pub fn other_env_key(&self) -> &'static str {
        match self {
            Self::AnthropicAuthToken => "ANTHROPIC_API_KEY",
            Self::AnthropicApiKey => "ANTHROPIC_AUTH_TOKEN",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum TargetKind {
    ClaudeCode,
    Codex,
    Pi,
    #[serde(alias = "prime_agent", alias = "prime-agent")]
    Prime,
    ZCode,
}

impl TargetKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude_code",
            Self::Codex => "codex",
            Self::Pi => "pi",
            Self::Prime => "prime",
            Self::ZCode => "zcode",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "claude_code" => Some(Self::ClaudeCode),
            "codex" => Some(Self::Codex),
            "pi" => Some(Self::Pi),
            "prime" | "prime_agent" | "prime-agent" => Some(Self::Prime),
            "zcode" => Some(Self::ZCode),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApplyStatus {
    Applied,
    Stale,
    Orphan,
    NotApplied,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SiteApiKeySummary {
    pub id: String,
    pub label: String,
    pub key_prefix: String,
    pub is_active: bool,
    pub quota_revision: String,
    pub selected_model_id: Option<String>,
    pub last_model_fetch_at: Option<i64>,
    pub last_model_fetch_latency_ms: Option<i64>,
    pub last_model_fetch_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SiteDto {
    pub id: String,
    pub name: String,
    pub base_url: String,
    #[serde(default)]
    pub base_urls: Vec<String>,
    pub key_prefix: String,
    pub quota_revision: String,
    pub has_key: bool,
    pub protocol: String,
    pub claude_auth_key_style: String,
    pub notes: Option<String>,
    pub enabled: bool,
    pub sort_order: i64,
    pub selected_model_id: Option<String>,
    pub last_model_fetch_at: Option<i64>,
    pub last_model_fetch_latency_ms: Option<i64>,
    pub last_model_fetch_error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(default)]
    pub capabilities: SiteCapabilities,
    #[serde(default)]
    pub active_api_key_id: Option<String>,
    #[serde(default)]
    pub api_keys: Vec<SiteApiKeySummary>,
    #[serde(default)]
    pub newapi_configured: bool,
    #[serde(default)]
    pub newapi_user_id: Option<String>,
    /// 已配置的代理请求头条数（明文计数，列表不返回请求头内容）。
    #[serde(default)]
    pub proxy_header_count: u32,
    /// ZCode 目标的 API 协议；`None` = 按站点协议推断。
    #[serde(default)]
    pub zcode_api_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SiteModelDto {
    pub id: String,
    pub site_id: String,
    #[serde(default)]
    pub api_key_id: String,
    pub model_id: String,
    pub display_name: String,
    pub owned_by: Option<String>,
    pub raw: Option<serde_json::Value>,
    #[serde(default)]
    pub is_manual: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddSiteApiKeyInput {
    pub label: Option<String>,
    pub api_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateSiteInput {
    pub name: String,
    #[serde(default)]
    pub base_url: String,
    pub base_urls: Option<Vec<String>>,
    pub api_key: String,
    #[serde(default)]
    pub api_key_label: Option<String>,
    #[serde(default)]
    pub extra_api_keys: Vec<AddSiteApiKeyInput>,
    pub protocol: Option<String>,
    pub claude_auth_key_style: Option<String>,
    pub notes: Option<String>,
    #[serde(default)]
    pub capabilities: Option<SiteCapabilities>,
    #[serde(default)]
    pub newapi_access_token: Option<String>,
    #[serde(default)]
    pub newapi_user_id: Option<String>,
    /// 本地代理请求头覆盖。`None` = 不改动既有值。
    #[serde(default)]
    pub proxy_headers: Option<Vec<ProxyHeader>>,
    /// ZCode 目标的 API 协议（`anthropic-messages` 等）。`None` = 按站点协议推断。
    #[serde(default)]
    pub zcode_api_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeepLinkSiteImportInput {
    pub name: String,
    pub base_urls: Vec<String>,
    pub api_key: String,
    pub protocol: Option<String>,
    pub notes: Option<String>,
    #[serde(default)]
    pub capabilities: Option<SiteCapabilities>,
    #[serde(default)]
    pub key_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeepLinkSiteImportResult {
    pub site: SiteDto,
    pub created: bool,
    pub added_api_key: bool,
    pub reused_api_key: bool,
    pub activated_api_key: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSiteApiKeyInput {
    pub label: Option<String>,
    pub api_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpsertSiteApiKeyInput {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
    pub api_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSiteInput {
    pub name: Option<String>,
    pub base_url: Option<String>,
    pub base_urls: Option<Vec<String>>,
    pub api_key: Option<String>,
    #[serde(default)]
    pub api_keys: Option<Vec<UpsertSiteApiKeyInput>>,
    pub protocol: Option<String>,
    pub claude_auth_key_style: Option<String>,
    pub notes: Option<String>,
    pub enabled: Option<bool>,
    pub selected_model_id: Option<String>,
    pub sort_order: Option<i64>,
    #[serde(default)]
    pub capabilities: Option<SiteCapabilities>,
    #[serde(default)]
    pub newapi_access_token: Option<String>,
    #[serde(default)]
    pub newapi_user_id: Option<String>,
    /// 本地代理请求头覆盖。`None` = 不改动既有值。
    #[serde(default)]
    pub proxy_headers: Option<Vec<ProxyHeader>>,
    /// ZCode 目标的 API 协议（`anthropic-messages` 等）。`None` = 按站点协议推断。
    #[serde(default)]
    pub zcode_api_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FetchModelsResult {
    pub models: Vec<SiteModelDto>,
    pub latency_ms: u64,
    pub endpoint: String,
    pub fetched_at: i64,
    #[serde(default)]
    pub api_key_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeSiteApiKeyResult {
    pub model_count: usize,
    pub latency_ms: u64,
    pub endpoint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolDetectionResult {
    pub detected_protocol: SiteProtocol,
    pub model_preview: Vec<SiteModelDto>,
    pub endpoint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelFetchOutcome {
    pub ok: bool,
    pub api_key_id: String,
    pub latency_ms: u64,
    pub endpoint: Option<String>,
    pub fetched_at: Option<i64>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelProbeResult {
    pub model_id: String,
    pub ok: bool,
    pub latency_ms: u64,
    pub status: Option<u16>,
    pub error: Option<String>,
    pub endpoint: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum QuotaProbeStatus {
    Available,
    Unsupported,
    Unauthorized,
    Error,
    #[serde(rename = "invalid_data")]
    InvalidData,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum QuotaSource {
    CreditGrants,
    SubscriptionUsage,
    SubscriptionOnly,
    UsageOnly,
    TokenUsage,
    UserSelf,
    OpencodeGo,
    /// Sub2API 站点的 `/v1/usage` 钱包余额。
    Sub2Api,
    /// 魔搭（ModelScope）魔粒账户余额，口径是点数而不是金额。
    MagicubeBalance,
}

/// One usage window of an OpenCode Go plan (5-hour / weekly / monthly).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct QuotaWindow {
    /// `rolling` (5-hour) | `weekly` | `monthly`.
    pub kind: String,
    /// Consumed percentage of the window limit, 0-100.
    pub usage_percent: Option<f64>,
    /// Absolute reset time in milliseconds since epoch.
    pub reset_at: Option<i64>,
    /// Window limit in USD, when the upstream reports it.
    pub limit_usd: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SiteQuota {
    pub status: QuotaProbeStatus,
    pub remaining_usd: Option<f64>,
    pub used_usd: Option<f64>,
    pub total_usd: Option<f64>,
    pub unlimited: bool,
    #[serde(default)]
    pub unit: Option<String>,
    pub expires_at: Option<i64>,
    pub source: Option<QuotaSource>,
    pub endpoint: Option<String>,
    pub fetched_at: i64,
    pub latency_ms: u64,
    pub error: Option<String>,
    /// OpenCode Go usage windows; empty for every other quota source.
    #[serde(default)]
    pub windows: Vec<QuotaWindow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetLiveStatus {
    pub kind: TargetKind,
    pub installed: bool,
    pub version: Option<String>,
    pub config_path: String,
    pub status: ApplyStatus,
    pub applied_site_id: Option<String>,
    pub applied_site_name: Option<String>,
    pub applied_model_id: Option<String>,
    pub provider_id: Option<String>,
    pub orphan: bool,
    pub live_summary: HashMap<String, Option<String>>,
    pub last_applied_at: Option<i64>,
    pub stale_reason: Option<String>,
}

/// Claude Code effort / thinking level persisted in settings.json.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ClaudeEffortLevel {
    Low,
    Medium,
    High,
    #[serde(alias = "max")]
    Xhigh,
}

impl ClaudeEffortLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Xhigh => "xhigh",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "low" => Some(Self::Low),
            "medium" => Some(Self::Medium),
            "high" => Some(Self::High),
            "xhigh" | "max" => Some(Self::Xhigh),
            _ => None,
        }
    }
}

/// Codex reasoning effort in config.toml.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CodexReasoningEffort {
    Minimal,
    Low,
    Medium,
    High,
    Xhigh,
}

impl CodexReasoningEffort {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Xhigh => "xhigh",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "minimal" => Some(Self::Minimal),
            "low" => Some(Self::Low),
            "medium" => Some(Self::Medium),
            "high" => Some(Self::High),
            "xhigh" => Some(Self::Xhigh),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CapabilitySource {
    #[default]
    Site,
    Custom,
}

impl CapabilitySource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Site => "site",
            Self::Custom => "custom",
        }
    }

    pub fn parse(raw: Option<&str>) -> Self {
        match raw.map(str::trim).map(|s| s.to_ascii_lowercase()) {
            Some(value) if value == "custom" => Self::Custom,
            _ => Self::Site,
        }
    }
}

/// Extra options for Claude Code apply.
#[derive(Debug, Clone, Default)]
pub struct ClaudeApplyOptions {
    pub fable_model_id: Option<String>,
    pub opus_model_id: Option<String>,
    pub sonnet_model_id: Option<String>,
    pub haiku_model_id: Option<String>,
    pub effort_level: Option<ClaudeEffortLevel>,
    pub use_1m_context: bool,
}

/// Extra options for Codex apply.
#[derive(Debug, Clone, Default)]
pub struct CodexApplyOptions {
    pub write_all_models: bool,
    pub reasoning_effort: Option<CodexReasoningEffort>,
    /// Site models used when `write_all_models` is true.
    pub catalog_models: Vec<(String, String)>, // (model_id, display_name)
    pub remote_compaction: bool,
    pub image_understanding: bool,
    pub image_generation: bool,
    pub web_search: bool,
    pub capability_source: CapabilitySource,
}

/// Extra options for Pi Coding Agent apply.
#[derive(Debug, Clone, Default)]
pub struct PiApplyOptions {
    pub write_all_models: bool,
    /// Site models used when `write_all_models` is true.
    pub catalog_models: Vec<(String, String)>, // (model_id, display_name)
    pub thinking: ThinkingWrite,
}

/// Extra options for Prime Agent apply.
#[derive(Debug, Clone, Default)]
pub struct PrimeApplyOptions {
    pub write_all_models: bool,
    /// Site models used when `write_all_models` is true.
    pub catalog_models: Vec<(String, String)>, // (model_id, display_name)
    pub thinking: ThinkingWrite,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyTargetResult {
    pub target: TargetKind,
    pub ok: bool,
    pub status: ApplyStatus,
    pub backup_paths: Vec<String>,
    pub message: String,
    pub live_summary: Option<HashMap<String, Option<String>>>,
    pub touched_keys: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyResult {
    pub site_id: String,
    pub model_id: String,
    pub results: Vec<ApplyTargetResult>,
    pub applied_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub language: String,
    pub theme_mode: String,
    pub primary_color: String,
    pub auto_start: bool,
    pub always_on_top: bool,
    pub claude_home_override: Option<String>,
    pub codex_home_override: Option<String>,
    #[serde(default)]
    pub pi_agent_dir_override: Option<String>,
    #[serde(default)]
    pub prime_agent_dir_override: Option<String>,
    /// ZCode 配置根目录覆盖（默认 `~/.zcode`，provider 在 `v2/`、MCP 在 `cli/`）。
    #[serde(default)]
    pub zcode_home_override: Option<String>,
    pub codex_env_inject_mode: String,
    pub force_exclusive_claude_auth_key: bool,
    #[serde(default = "default_true")]
    pub auto_check_update: bool,
    #[serde(default = "default_update_check_interval")]
    pub update_check_interval: u32,
    /// Max backup copies kept per target under ~/.xiaobai-switch/backups/{target}.
    #[serde(default = "default_max_backup_copies")]
    pub max_backup_copies: u32,
    #[serde(default = "default_proxy_mode")]
    pub proxy_mode: String,
    #[serde(default = "default_proxy_protocol")]
    pub proxy_protocol: String,
    #[serde(default)]
    pub proxy_host: Option<String>,
    #[serde(default)]
    pub proxy_port: Option<u16>,
    #[serde(default = "default_route_probe_ttl")]
    pub route_probe_ttl_minutes: u32,
    /// 本地代理总开关（运行意图；应用启动时按它自动拉起）。
    #[serde(default)]
    pub local_proxy_enabled: bool,
    #[serde(default = "default_local_proxy_port")]
    pub local_proxy_port: u16,
    /// 接管目标集合：列在这里的目标写入客户端的 Base URL 指向本地代理。
    #[serde(default)]
    pub local_proxy_targets: Vec<TargetKind>,
    /// Hide the main window instead of quitting when the user closes it.
    #[serde(default = "default_true")]
    pub close_to_tray: bool,
    /// Keep the main window hidden on launch (only meaningful with close_to_tray).
    #[serde(default)]
    pub start_in_tray: bool,
    /// 悬浮窗配置
    #[serde(default)]
    pub floating_window: FloatingWindowSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FloatingWindowSettings {
    /// 是否启用悬浮窗
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// 自动刷新间隔（分钟）
    #[serde(default = "default_floating_refresh_interval")]
    pub auto_refresh_minutes: u32,
    /// 窗口 X 坐标（None = 使用默认位置）
    #[serde(default)]
    pub position_x: Option<i32>,
    /// 窗口 Y 坐标（None = 使用默认位置）
    #[serde(default)]
    pub position_y: Option<i32>,
    /// 是否收起（true = 只显示图标，false = 显示完整列表）
    #[serde(default)]
    pub collapsed: bool,
}

impl Default for FloatingWindowSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            auto_refresh_minutes: default_floating_refresh_interval(),
            position_x: None,
            position_y: None,
            collapsed: false,
        }
    }
}

pub fn default_floating_refresh_interval() -> u32 {
    5
}

pub fn clamp_floating_refresh_interval(n: u32) -> u32 {
    n.clamp(1, 60)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebDavConfigView {
    pub base_url: String,
    pub username: String,
    pub remote_path: String,
    pub accept_invalid_certs: bool,
    pub has_password: bool,
    pub auto_sync_enabled: bool,
    pub sync_interval_minutes: u32,
    pub max_remote_backups: u32,
}

impl Default for WebDavConfigView {
    fn default() -> Self {
        Self {
            base_url: String::new(),
            username: String::new(),
            // 远端目录名属于跨机内部协议路径：与同步 manifest 名称绑定，
            // 改名会让新旧版本机器落到不同目录而互相看不到对方数据。
            remote_path: "xiaobai-switch".into(),
            accept_invalid_certs: false,
            has_password: false,
            auto_sync_enabled: false,
            sync_interval_minutes: 60,
            max_remote_backups: 3,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveWebDavConfigInput {
    pub base_url: String,
    pub username: String,
    #[serde(default)]
    pub password: Option<String>,
    pub remote_path: String,
    #[serde(default)]
    pub accept_invalid_certs: bool,
    #[serde(default)]
    pub auto_sync_enabled: bool,
    pub sync_interval_minutes: u32,
    pub max_remote_backups: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestWebDavConnectionInput {
    pub base_url: String,
    pub username: String,
    #[serde(default)]
    pub password: Option<String>,
    pub remote_path: String,
    #[serde(default)]
    pub accept_invalid_certs: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteBackupInfo {
    pub file_name: String,
    pub size: u64,
    pub last_modified: String,
    pub device_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalBackupInfo {
    pub file_name: String,
    pub size: u64,
    pub created_at: i64,
    pub device_name: String,
    pub reason: Option<String>,
    pub app_version: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebDavSyncStatus {
    pub last_attempt_at: Option<i64>,
    pub last_success_at: Option<i64>,
    pub status: String,
    pub error: Option<String>,
}

impl Default for WebDavSyncStatus {
    fn default() -> Self {
        Self {
            last_attempt_at: None,
            last_success_at: None,
            status: "never".into(),
            error: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupOverview {
    pub latest_local_backup_at: Option<i64>,
    pub webdav_configured: bool,
    pub webdav_auto_sync_enabled: bool,
    pub webdav_sync: WebDavSyncStatus,
    pub next_scheduled_at: Option<i64>,
    pub sync_revision: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupOperationResult {
    pub file_name: String,
    pub local_path: Option<String>,
    pub uploaded: bool,
    pub warning: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreStartupResult {
    pub status: String,
    pub message: String,
}

/// 一次同步引擎运行的结果。
/// action: "upload"（本机数据发布到远端）/ "download"（应用远端数据，需重启生效）/ "in_sync"（两侧一致）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncOutcome {
    pub action: String,
    pub revision: u64,
    pub bundle_file_name: Option<String>,
    pub conflict: bool,
    pub pending_restart: bool,
    pub warning: Option<String>,
}

pub fn default_max_backup_copies() -> u32 {
    30
}

pub fn clamp_max_backup_copies(n: u32) -> u32 {
    n.clamp(1, 200)
}

pub fn default_proxy_mode() -> String {
    "system".into()
}

pub fn default_proxy_protocol() -> String {
    "http".into()
}

pub fn default_route_probe_ttl() -> u32 {
    10
}

pub fn default_local_proxy_port() -> u16 {
    18087
}

/// 端口限制在非特权区间，避免与系统服务抢端口。
pub fn clamp_local_proxy_port(port: u16) -> u16 {
    port.clamp(1024, 65535)
}

/// 接管目标去重并保持用户勾选顺序；顺序只影响 UI 展示。
pub fn normalize_local_proxy_targets(targets: Vec<TargetKind>) -> Vec<TargetKind> {
    let mut out: Vec<TargetKind> = Vec::new();
    for target in targets {
        if !out.contains(&target) {
            out.push(target);
        }
    }
    out
}

/// 路径口令：32 位十六进制随机串，用于把"能用这个代理"限制在读过客户端配置的
/// 进程。持久化在设备本地文件（见 `paths::ensure_local_proxy_token`），不进 settings
/// ——settings 会随 WebDAV 同步到别的机器，而口令必须与本机 CLI 配置一致。
pub fn generate_local_proxy_token() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

pub fn is_valid_local_proxy_token(token: &str) -> bool {
    token.len() == 32 && token.chars().all(|c| c.is_ascii_hexdigit())
}

pub fn clamp_route_probe_ttl(n: u32) -> u32 {
    n.clamp(1, 1440)
}

pub fn default_update_check_interval() -> u32 {
    60
}

pub fn clamp_update_check_interval(n: u32) -> u32 {
    n.clamp(1, 1440)
}

pub fn default_true() -> bool {
    true
}

pub fn normalize_proxy_mode(s: &str) -> String {
    match s.trim() {
        "none" | "custom" => s.trim().into(),
        _ => default_proxy_mode(),
    }
}

pub fn normalize_proxy_protocol(s: &str) -> String {
    match s.trim().to_ascii_lowercase().as_str() {
        "https" | "socks5" => s.trim().to_ascii_lowercase(),
        _ => default_proxy_protocol(),
    }
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            language: "zh-CN".into(),
            theme_mode: "system".into(),
            primary_color: "#1677ff".into(),
            auto_start: false,
            always_on_top: false,
            claude_home_override: None,
            codex_home_override: None,
            pi_agent_dir_override: None,
            prime_agent_dir_override: None,
            zcode_home_override: None,
            codex_env_inject_mode: "auto".into(),
            force_exclusive_claude_auth_key: false,
            auto_check_update: true,
            update_check_interval: default_update_check_interval(),
            max_backup_copies: default_max_backup_copies(),
            proxy_mode: default_proxy_mode(),
            proxy_protocol: default_proxy_protocol(),
            proxy_host: None,
            proxy_port: None,
            route_probe_ttl_minutes: default_route_probe_ttl(),
            local_proxy_enabled: false,
            local_proxy_port: default_local_proxy_port(),
            local_proxy_targets: Vec::new(),
            close_to_tray: true,
            start_in_tray: false,
            floating_window: FloatingWindowSettings::default(),
        }
    }
}

/// 悬浮窗显示的站点余额摘要
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SiteQuotaSummary {
    pub site_id: String,
    pub site_name: String,
    pub quota: Option<SiteQuota>,
    pub enabled: bool,
    pub sort_order: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SwitchRouteResult {
    pub site: SiteDto,
    pub results: Vec<ApplyTargetResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SwitchSiteApiKeyResult {
    pub site: SiteDto,
    pub models: Vec<SiteModelDto>,
    pub fetch: ModelFetchOutcome,
    pub results: Vec<ApplyTargetResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UrlProbeResult {
    pub url: String,
    pub ok: bool,
    pub latency_ms: u64,
    pub status: Option<u16>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HttpBytesResult {
    pub status: u16,
    pub content_type: String,
    pub final_url: String,
    pub base64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyRecordDto {
    pub id: String,
    pub site_id: Option<String>,
    pub site_name_snapshot: String,
    pub target: String,
    pub model_id: String,
    pub provider_id: Option<String>,
    pub status: String,
    pub backup_dir: Option<String>,
    pub error: Option<String>,
    pub applied_at: i64,
    #[serde(default)]
    pub site_api_key_id: Option<String>,
    #[serde(default)]
    pub site_api_key_label_snapshot: Option<String>,
    #[serde(default)]
    pub site_api_key_prefix_snapshot: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupInfo {
    pub id: String,
    pub target: String,
    pub dir: String,
    pub created_at: i64,
    pub files: Vec<String>,
    pub apply_record_id: Option<String>,
    pub site_name_snapshot: Option<String>,
    #[serde(default)]
    pub model_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupFileInfo {
    pub name: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupPreview {
    pub id: String,
    pub summary: HashMap<String, Option<String>>,
    pub files: Vec<BackupFileInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CliToolInfo {
    pub kind: TargetKind,
    pub installed: bool,
    pub version: Option<String>,
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TouchedKeys {
    pub paths: Vec<String>,
    pub created_paths: Vec<String>,
    pub env_keys: Vec<String>,
    pub claude_env_keys: Vec<String>,
    pub shell_rc_paths: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default)]
pub struct BindingApiKeySnapshot {
    pub id: Option<String>,
    pub label: Option<String>,
    pub prefix: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TargetBinding {
    pub target: TargetKind,
    pub site_id: Option<String>,
    pub site_name_snapshot: String,
    pub model_id: String,
    pub provider_id: Option<String>,
    pub key_fingerprint: String,
    pub managed_paths: Vec<String>,
    pub managed_env_keys: Vec<String>,
    pub expected_fields: HashMap<String, String>,
    pub orphan: bool,
    pub applied_at: i64,
    pub apply_record_id: Option<String>,
    pub api_key: BindingApiKeySnapshot,
}

#[derive(Debug, Clone, Default)]
pub struct SiteKeyState {
    pub active_api_key_id: Option<String>,
    pub api_keys: Vec<SiteApiKeySummary>,
}

#[derive(Debug, Clone)]
pub struct SiteRow {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub base_urls: Vec<String>,
    pub api_key_encrypted: String,
    pub key_prefix: String,
    pub protocol: SiteProtocol,
    pub claude_auth_key_style: ClaudeAuthKeyStyle,
    pub notes: Option<String>,
    pub enabled: bool,
    pub sort_order: i64,
    pub selected_model_id: Option<String>,
    pub last_model_fetch_at: Option<i64>,
    pub last_model_fetch_latency_ms: Option<i64>,
    pub last_model_fetch_error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub capabilities: SiteCapabilities,
    pub keys: SiteKeyState,
    pub newapi_access_token_encrypted: Option<String>,
    pub newapi_user_id: Option<String>,
    /// 加密存储的 `Vec<ProxyHeader>` JSON；UI 只在编辑时按需解密。
    pub proxy_headers_encrypted: Option<String>,
    pub proxy_header_count: u32,
    /// ZCode 目标使用的 API 协议；未设置时按 `protocol` 推断。
    pub zcode_api_type: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SiteApiKeyRow {
    pub id: String,
    pub site_id: String,
    pub label: String,
    pub api_key_encrypted: String,
    pub key_prefix: String,
    pub is_active: bool,
    pub selected_model_id: Option<String>,
    pub last_model_fetch_at: Option<i64>,
    pub last_model_fetch_latency_ms: Option<i64>,
    pub last_model_fetch_error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl SiteApiKeyRow {
    pub fn to_summary(&self) -> SiteApiKeySummary {
        SiteApiKeySummary {
            id: self.id.clone(),
            label: self.label.clone(),
            key_prefix: self.key_prefix.clone(),
            is_active: self.is_active,
            quota_revision: key_fingerprint(&self.api_key_encrypted),
            selected_model_id: self.selected_model_id.clone(),
            last_model_fetch_at: self.last_model_fetch_at,
            last_model_fetch_latency_ms: self.last_model_fetch_latency_ms,
            last_model_fetch_error: self.last_model_fetch_error.clone(),
        }
    }
}

impl SiteRow {
    pub fn to_dto(&self) -> SiteDto {
        SiteDto {
            id: self.id.clone(),
            name: self.name.clone(),
            base_url: self.base_url.clone(),
            base_urls: if self.base_urls.is_empty() {
                vec![self.base_url.clone()]
            } else {
                self.base_urls.clone()
            },
            key_prefix: self.key_prefix.clone(),
            quota_revision: key_fingerprint(&self.api_key_encrypted),
            has_key: !self.api_key_encrypted.is_empty(),
            protocol: self.protocol.as_str().into(),
            claude_auth_key_style: self.claude_auth_key_style.as_str().into(),
            notes: self.notes.clone(),
            enabled: self.enabled,
            sort_order: self.sort_order,
            selected_model_id: self.selected_model_id.clone(),
            last_model_fetch_at: self.last_model_fetch_at,
            last_model_fetch_latency_ms: self.last_model_fetch_latency_ms,
            last_model_fetch_error: self.last_model_fetch_error.clone(),
            created_at: self.created_at,
            updated_at: self.updated_at,
            capabilities: self.capabilities.clone(),
            active_api_key_id: self.keys.active_api_key_id.clone(),
            api_keys: self.keys.api_keys.clone(),
            newapi_configured: self
                .newapi_access_token_encrypted
                .as_deref()
                .is_some_and(|token| !token.is_empty()),
            newapi_user_id: self.newapi_user_id.clone(),
            proxy_header_count: self.proxy_header_count,
            zcode_api_type: self.zcode_api_type.clone(),
        }
    }

    pub fn active_api_key_id(&self) -> Option<&str> {
        self.keys.active_api_key_id.as_deref()
    }

    pub fn api_key_snapshot(&self) -> BindingApiKeySnapshot {
        let active = self
            .keys
            .api_keys
            .iter()
            .find(|k| k.is_active)
            .or_else(|| self.keys.api_keys.first());
        BindingApiKeySnapshot {
            id: self.keys.active_api_key_id.clone(),
            label: active.map(|k| k.label.clone()),
            prefix: Some(self.key_prefix.clone()).filter(|s| !s.is_empty()),
        }
    }
}

pub fn provider_id_for_site(site_id: &str) -> String {
    let compact: String = site_id.chars().filter(|c| *c != '-').collect();
    let short = if compact.len() >= 12 {
        compact[..12].to_lowercase()
    } else {
        compact.to_lowercase()
    };
    format!("xiaobai_{short}")
}

pub fn env_key_for_site(site_id: &str) -> String {
    let compact: String = site_id.chars().filter(|c| *c != '-').collect();
    let short = if compact.len() >= 12 {
        compact[..12].to_uppercase()
    } else {
        compact.to_uppercase()
    };
    format!("XIAOBAI_SITE_{short}_API_KEY")
}
