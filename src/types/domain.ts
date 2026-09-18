import type { ProxyHeader } from "./proxy";

/** Model list / probe protocol */
export type SiteProtocol = "openai_compatible" | "anthropic";

/** Only affects Claude Code auth env key name. Codex ignores this. */
export type ClaudeAuthKeyStyle = "anthropic_auth_token" | "anthropic_api_key";

/**
 * ZCode provider 的连接协议。ZCode 把供应商拆成两份文件，`kind` 与 `api.type`
 * 必须配套；该值无法由 `SiteProtocol` 可靠推断，所以让用户显式选择。
 */
export type ZCodeApiType = "anthropic-messages" | "openai-responses" | "openai-chat-completions";

export type TargetKind = "claude_code" | "codex" | "pi" | "prime" | "zcode";

/**
 * 技能安装目标：只支持能放 SKILL.md 的四个客户端（ZCode 未接入技能）。
 * 与后端 `commands::skills::SkillTarget` 的 5 个取值（含 `agents`）保持一致。
 */
export type SkillTarget = "agents" | "claude_code" | "codex" | "pi" | "prime";

export interface Skill {
  name: string;
  description: string;
  author: string | null;
  version: string | null;
  target: SkillTarget;
  /** Actual SKILL.md or SKILL.md.disabled path. */
  sourcePath: string;
  /** Resolved root directory for this target's skills. */
  skillsPath: string;
  /** Directory containing this skill. */
  directoryPath: string;
  enabled: boolean;
}

export interface SkillDetail {
  info: Skill;
  content: string;
  files: string[];
}

export interface MarketplaceSkill {
  name: string;
  description: string;
  repo: string;
  stars: number;
  installs: number;
  installedTargets: SkillTarget[];
}

export type ProxyMode = "system" | "none" | "custom";
export type ProxyProtocol = "http" | "https" | "socks5";

export type ApplyStatus = "applied" | "stale" | "orphan" | "not_applied" | "failed";

/** Kebab capability flags shared by site JSON and xiaobaiswitchplus:// query keys. */
export type SiteCapabilities = Record<string, boolean>;

export type CodexCapabilitySource = "site" | "custom";

export interface AppError {
  code:
    | "network"
    | "timeout"
    | "unauthorized"
    | "not_found"
    | "invalid_response"
    | "ssl"
    | "atomic_write_failed"
    | "backup_failed"
    | "validation_failed"
    | "lock_busy"
    | "master_key_missing"
    | "invalid_config"
    | "internal"
    | "autostart_failed"
    | "webdav_not_configured"
    | "webdav_auth_failed"
    | "backup_invalid"
    | "restore_pending"
    | "sync_algorithm_mismatch";
  message: string;
  details?: string | null;
}

export interface SiteApiKeySummary {
  id: string;
  label: string;
  keyPrefix: string;
  isActive: boolean;
  quotaRevision: string;
  selectedModelId: string | null;
  lastModelFetchAt: number | null;
  lastModelFetchLatencyMs: number | null;
  lastModelFetchError: string | null;
}

export interface Site {
  id: string;
  name: string;
  baseUrl: string;
  baseUrls: string[];
  keyPrefix: string;
  /** Opaque revision of the stored credential, never the credential itself. */
  quotaRevision: string;
  hasKey: boolean;
  protocol: SiteProtocol;
  claudeAuthKeyStyle: ClaudeAuthKeyStyle;
  notes: string | null;
  enabled: boolean;
  sortOrder: number;
  selectedModelId: string | null;
  lastModelFetchAt: number | null;
  lastModelFetchLatencyMs: number | null;
  lastModelFetchError: string | null;
  createdAt: number;
  updatedAt: number;
  capabilities?: SiteCapabilities;
  activeApiKeyId?: string | null;
  apiKeys?: SiteApiKeySummary[];
  /** NewAPI 访问令牌是否已配置（令牌本身不回传前端）。 */
  newapiConfigured?: boolean;
  newapiUserId?: string | null;
  /** 已配置的代理请求头条数（不透出请求头内容）。 */
  proxyHeaderCount?: number;
  /** ZCode 目标的 API 协议；null = 按站点协议推断。 */
  zcodeApiType?: ZCodeApiType | null;
}

export interface SiteModel {
  id: string;
  siteId: string;
  apiKeyId?: string;
  modelId: string;
  displayName: string;
  ownedBy: string | null;
  raw: Record<string, unknown> | null;
  isManual?: boolean;
}

export interface DeepLinkSiteImportInput {
  name: string;
  baseUrls: string[];
  apiKey: string;
  protocol?: SiteProtocol;
  notes?: string | null;
  capabilities?: SiteCapabilities;
  keyName?: string | null;
}

export interface DeepLinkSiteImportResult {
  site: Site;
  created: boolean;
  addedApiKey: boolean;
  reusedApiKey: boolean;
  activatedApiKey: boolean;
}

export interface CreateSiteInput {
  name: string;
  baseUrl?: string;
  baseUrls?: string[];
  apiKey: string;
  apiKeyLabel?: string | null;
  extraApiKeys?: AddSiteApiKeyInput[];
  protocol?: SiteProtocol;
  claudeAuthKeyStyle?: ClaudeAuthKeyStyle;
  notes?: string | null;
  capabilities?: SiteCapabilities;
  newapiAccessToken?: string | null;
  newapiUserId?: string | null;
  /** ZCode 目标的 API 协议；缺省 = 按站点协议推断。 */
  zcodeApiType?: ZCodeApiType | null;
  /** 本地代理请求头覆盖；缺省表示不改动。 */
  proxyHeaders?: ProxyHeader[];
}

export interface UpsertSiteApiKeyInput {
  id?: string | null;
  label?: string | null;
  apiKey: string;
}

export interface UpdateSiteInput {
  name?: string;
  baseUrl?: string;
  baseUrls?: string[];
  apiKey?: string | null;
  apiKeys?: UpsertSiteApiKeyInput[] | null;
  protocol?: SiteProtocol;
  claudeAuthKeyStyle?: ClaudeAuthKeyStyle;
  notes?: string | null;
  enabled?: boolean;
  selectedModelId?: string | null;
  sortOrder?: number;
  capabilities?: SiteCapabilities;
  newapiAccessToken?: string | null;
  newapiUserId?: string | null;
  /** ZCode 目标的 API 协议；缺省不改动，空串表示清除（回到按协议推断）。 */
  zcodeApiType?: ZCodeApiType | null;
  /** 本地代理请求头覆盖；缺省表示不改动。 */
  proxyHeaders?: ProxyHeader[];
}

export interface FetchModelsResult {
  models: SiteModel[];
  latencyMs: number;
  endpoint: string;
  fetchedAt: number;
  apiKeyId?: string;
}

export interface RefreshSiteResult {
  siteId: string;
  modelCount: number;
  modelsOk: boolean;
  quotaOk: boolean;
  modelError: string | null;
  quotaError: string | null;
}

export interface RefreshAllSitesResult {
  sites: RefreshSiteResult[];
  successCount: number;
  failureCount: number;
}

export interface ProbeSiteApiKeyResult {
  modelCount: number;
  latencyMs: number;
  endpoint: string;
}

export interface ProtocolDetectionResult {
  detectedProtocol: SiteProtocol;
  modelPreview: SiteModel[];
  endpoint: string;
}

export interface ModelFetchOutcome {
  ok: boolean;
  apiKeyId: string;
  latencyMs: number;
  endpoint: string | null;
  fetchedAt: number | null;
  error: string | null;
}

export interface AddSiteApiKeyInput {
  label?: string | null;
  apiKey: string;
}

export interface UpdateSiteApiKeyInput {
  label?: string | null;
  apiKey?: string | null;
}

export interface SwitchSiteApiKeyResult {
  site: Site;
  models: SiteModel[];
  fetch: ModelFetchOutcome;
  results: ApplyTargetResult[];
}

export type LiveSummary = Record<string, string | null>;

export interface TargetLiveStatus {
  kind: TargetKind;
  installed: boolean;
  version: string | null;
  configPath: string;
  status: ApplyStatus;
  appliedSiteId: string | null;
  appliedSiteName: string | null;
  appliedModelId: string | null;
  providerId: string | null;
  orphan: boolean;
  liveSummary: LiveSummary;
  lastAppliedAt: number | null;
  staleReason: string | null;
}

/** Claude Code effort / thinking level */
export type ClaudeEffortLevel = "low" | "medium" | "high" | "xhigh";

/** Codex reasoning effort in config.toml */
export type CodexReasoningEffort = "minimal" | "low" | "medium" | "high" | "xhigh";

export type ThinkingTarget = "pi" | "prime";
export type ThinkingLevel = "off" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max";

export interface ModelThinkingConfig {
  reasoning: boolean;
  forceAdaptiveThinking?: boolean;
  thinkingLevelMap?: Record<string, string>;
}

export interface ThinkingExtendedMap {
  xhigh?: string | null;
  max?: string | null;
}

export interface SiteThinkingPreset {
  siteId: string;
  target: ThinkingTarget;
  defaultLevel?: ThinkingLevel | null;
  extended: ThinkingExtendedMap;
  models: Record<string, ModelThinkingConfig>;
}

export interface ApplyRequest {
  siteId: string;
  apiKeyId?: string;
  targets: TargetKind[];
  modelId: string;
  claudeAuthKeyStyle?: ClaudeAuthKeyStyle;
  /** Maps Claude Code "opus" alias to a site model id */
  claudeOpusModelId?: string | null;
  /** Maps Claude Code "sonnet" alias to a site model id */
  claudeSonnetModelId?: string | null;
  /** Maps Claude Code "haiku" alias to a site model id */
  claudeHaikuModelId?: string | null;
  /** Maps Claude Code "fable" alias to a site model id */
  claudeFableModelId?: string | null;
  claudeEffortLevel?: ClaudeEffortLevel | null;
  /** Append Claude Code's official `[1m]` declaration to compatible model ids. */
  claudeUse1mContext?: boolean;
  /** Write site model list into Codex model catalog for switching */
  codexWriteAllModels?: boolean;
  codexReasoningEffort?: CodexReasoningEffort | null;
  /** Codex: enable remote history compaction for the current provider */
  codexRemoteCompaction?: boolean;
  /** Codex: allow sending local images to the model */
  codexImageUnderstanding?: boolean;
  /** Codex: allow the hosted image generation tool */
  codexImageGeneration?: boolean;
  /** Codex: allow the hosted web_search tool */
  codexWebSearch?: boolean;
  /** `site` reads the site preset; `custom` uses the four bools above. */
  codexCapabilitySource?: CodexCapabilitySource;
  /** Write the site model list into Pi's managed provider. */
  piWriteAllModels?: boolean;
  /** Write the site model list into Prime's managed provider. */
  primeWriteAllModels?: boolean;
  /** Write the site model list into ZCode's managed providers. */
  zcodeWriteAllModels?: boolean;
}

export interface ApplyTargetResult {
  target: TargetKind;
  ok: boolean;
  status: ApplyStatus;
  backupPaths: string[];
  message: string;
  liveSummary?: LiveSummary;
  touchedKeys?: string[];
}

export interface ApplyResult {
  siteId: string;
  modelId: string;
  results: ApplyTargetResult[];
  appliedAt: number;
}

export interface AppSettings {
  language: "zh-CN" | "en-US";
  themeMode: "system" | "light" | "dark";
  primaryColor: string;
  autoStart: boolean;
  alwaysOnTop: boolean;
  claudeHomeOverride: string | null;
  codexHomeOverride: string | null;
  piAgentDirOverride: string | null;
  primeAgentDirOverride: string | null;
  /** ZCode 配置根目录覆盖（默认 `~/.zcode`）。 */
  zcodeHomeOverride: string | null;
  codexEnvInjectMode: "auto" | "shell_rc" | "user_env" | "file_only";
  forceExclusiveClaudeAuthKey: boolean;
  autoCheckUpdate: boolean;
  /** Auto update check interval in minutes. Default 60. */
  updateCheckInterval: number;
  /** Max backup copies kept per target. Default 30. */
  maxBackupCopies: number;
  proxyMode: ProxyMode;
  proxyProtocol: ProxyProtocol;
  proxyHost: string | null;
  proxyPort: number | null;
  routeProbeTtlMinutes: number;
  /** 本地代理总开关；应用启动时按它自动拉起代理。 */
  localProxyEnabled: boolean;
  /** 本地代理监听端口（仅回环），默认 18087。 */
  localProxyPort: number;
  /** 接管目标集合：这些目标写入客户端的 Base URL 指向本地代理。 */
  localProxyTargets: TargetKind[];
  /** Hide to the menu bar / tray instead of quitting on window close. */
  closeToTray: boolean;
  /** Keep the main window hidden on launch. Disabled when closeToTray is off. */
  startInTray: boolean;
  /** Floating window settings */
  floatingWindow?: {
    enabled: boolean;
    /** 自动刷新间隔（分钟）。与后端 `auto_refresh_minutes` 同名同单位。 */
    autoRefreshMinutes: number;
    positionX: number;
    positionY: number;
    collapsed: boolean;
  };
}

export interface WebDavConfigView {
  baseUrl: string;
  username: string;
  remotePath: string;
  acceptInvalidCerts: boolean;
  hasPassword: boolean;
  autoSyncEnabled: boolean;
  syncIntervalMinutes: number;
  maxRemoteBackups: number;
}

export interface SaveWebDavConfigInput {
  baseUrl: string;
  username: string;
  password?: string | null;
  remotePath: string;
  acceptInvalidCerts: boolean;
  autoSyncEnabled: boolean;
  syncIntervalMinutes: number;
  maxRemoteBackups: number;
}

export interface TestWebDavConnectionInput {
  baseUrl: string;
  username: string;
  password?: string | null;
  remotePath: string;
  acceptInvalidCerts: boolean;
}

export interface RemoteBackupInfo {
  fileName: string;
  size: number;
  lastModified: string;
  deviceName: string;
}

export interface LocalBackupInfo {
  fileName: string;
  size: number;
  createdAt: number;
  deviceName: string;
  reason: string | null;
  appVersion: string | null;
  error: string | null;
}

export interface WebDavSyncStatus {
  lastAttemptAt: number | null;
  lastSuccessAt: number | null;
  status: "never" | "running" | "success" | "warning" | "failed";
  error: string | null;
}

export interface BackupOverview {
  latestLocalBackupAt: number | null;
  webdavConfigured: boolean;
  webdavAutoSyncEnabled: boolean;
  webdavSync: WebDavSyncStatus;
  nextScheduledAt: number | null;
  syncRevision: number | null;
}

export interface BackupOperationResult {
  fileName: string;
  localPath: string | null;
  uploaded: boolean;
  warning: string | null;
}

export interface SyncOutcome {
  action: "upload" | "download" | "in_sync";
  revision: number;
  bundleFileName: string | null;
  conflict: boolean;
  pendingRestart: boolean;
  warning: string | null;
}

/** NewAPI 访问令牌连通性测试结果。 */
export interface NewApiAccessProbe {
  ok: boolean;
  status: number;
  remainingUsd: number | null;
  usedUsd: number | null;
  totalUsd: number | null;
  /** 与主界面额度行同源的货币单位（站点自报）；失败时为 null。 */
  unit?: string | null;
  endpoint: string;
  message: string | null;
}

export interface RestoreStartupResult {
  status: "applied" | "failed";
  message: string;
}

export interface SwitchRouteResult {
  site: Site;
  results: ApplyTargetResult[];
}

export interface UrlProbeResult {
  url: string;
  ok: boolean;
  latencyMs: number;
  status?: number | null;
  error?: string | null;
}

export interface ModelProbeResult {
  modelId: string;
  ok: boolean;
  latencyMs: number;
  status?: number | null;
  error?: string | null;
  endpoint: string;
}

export type QuotaProbeStatus =
  | "available"
  | "unsupported"
  | "unauthorized"
  | "invalid_data"
  | "error";

export type QuotaSource =
  | "credit_grants"
  | "subscription_usage"
  | "subscription_only"
  | "usage_only"
  | "token_usage"
  | "user_self"
  | "opencode_go"
  | "sub2_api"
  | "magicube_balance";

/** One OpenCode Go usage window (5-hour / weekly / monthly). */
export interface QuotaWindow {
  /** "rolling" (5-hour) | "weekly" | "monthly". */
  kind: string;
  /** Consumed percentage of the window limit, 0-100. */
  usagePercent: number | null;
  /** Absolute reset time in milliseconds since epoch. */
  resetAt: number | null;
  /** Window limit in USD, when reported upstream. */
  limitUsd: number | null;
}

export interface SiteQuota {
  status: QuotaProbeStatus;
  remainingUsd: number | null;
  usedUsd: number | null;
  totalUsd: number | null;
  unlimited: boolean;
  unit?: string | null;
  expiresAt: number | null;
  source: QuotaSource | null;
  endpoint: string | null;
  fetchedAt: number;
  latencyMs: number;
  error: string | null;
  /** OpenCode Go usage windows; empty/absent for other quota sources. */
  windows?: QuotaWindow[];
}

export interface SiteQuotaSummary {
  siteId: string;
  siteName: string;
  enabled: boolean;
  sortOrder: number;
  quota: SiteQuota | null;
}

export interface HttpBytesResult {
  status: number;
  contentType: string;
  finalUrl: string;
  base64: string;
}

export interface ApplyRecord {
  id: string;
  siteId: string | null;
  siteNameSnapshot: string;
  target: TargetKind;
  modelId: string;
  providerId: string | null;
  status: "success" | "failed" | "rolled_back";
  backupDir: string | null;
  error: string | null;
  appliedAt: number;
}

export interface BackupInfo {
  id: string;
  target: TargetKind;
  dir: string;
  createdAt: number;
  files: string[];
  applyRecordId: string | null;
  siteNameSnapshot: string | null;
  modelId?: string | null;
}

export interface BackupFileInfo {
  name: string;
  path: string;
}

export interface BackupPreview {
  id: string;
  summary: LiveSummary;
  files: BackupFileInfo[];
}

export interface CliToolInfo {
  kind: TargetKind;
  installed: boolean;
  version: string | null;
  path: string | null;
}

export interface AppPaths {
  appDir: string;
  dbPath: string;
  masterKeyPath: string;
  backupsDir: string;
  appBackupsDir: string;
  codexEnvPath: string;
  logsDir: string;
}

export interface UrlWritePreview {
  modelsUrl: string;
  claudeBaseUrl: string;
  codexBaseUrl: string;
}
