import type {
  AppPaths,
  AppSettings,
  ApplyResult,
  BackupOperationResult,
  BackupOverview,
  BackupInfo,
  BackupPreview,
  CliToolInfo,
  CreateSiteInput,
  DeepLinkSiteImportInput,
  DeepLinkSiteImportResult,
  FetchModelsResult,
  HttpBytesResult,
  LocalBackupInfo,
  ModelProbeResult,
  ProbeSiteApiKeyResult,
  MarketplaceSkill,
  RemoteBackupInfo,
  Skill,
  SkillTarget,
  Site,
  SiteApiKeySummary,
  SiteQuota,
  SiteModel,
  SiteThinkingPreset,
  SwitchRouteResult,
  SwitchSiteApiKeyResult,
  AddSiteApiKeyInput,
  UpdateSiteApiKeyInput,
  SyncOutcome,
  TargetKind,
  TargetLiveStatus,
  UpdateSiteInput,
  UrlProbeResult,
  WebDavConfigView,
} from "@/types/domain";
import type {
  McpApplyResult,
  McpApplyTargetResult,
  McpImportLocator,
  McpImportResult,
  McpServer,
  McpServerInput,
  RegistryCandidate,
  ScanOutcome,
  ScannedMcp,
} from "@/types/mcp";
import type { AgentRules, AgentRulesApplyResult } from "@/types/rules";
import type { LocalProxyRequestLogEntry, LocalProxyStatus, ProxyHeader } from "@/types/proxy";
import { keyPrefix, normalizeBaseUrl } from "./urlNormalize";

const DEFAULT_SETTINGS: AppSettings = {
  language: "zh-CN",
  themeMode: "system",
  primaryColor: "#1677ff",
  autoStart: false,
  alwaysOnTop: false,
  claudeHomeOverride: null,
  codexHomeOverride: null,
  piAgentDirOverride: null,
  primeAgentDirOverride: null,
  zcodeHomeOverride: null,
  codexEnvInjectMode: "auto",
  forceExclusiveClaudeAuthKey: false,
  autoCheckUpdate: true,
  updateCheckInterval: 60,
  maxBackupCopies: 30,
  proxyMode: "system",
  proxyProtocol: "http",
  proxyHost: null,
  proxyPort: null,
  routeProbeTtlMinutes: 10,
  localProxyEnabled: false,
  localProxyPort: 18087,
  localProxyTargets: [],
  closeToTray: true,
  startInTray: false,
  floatingWindow: {
    enabled: true,
    autoRefreshMinutes: 5,
    positionX: 100,
    positionY: 100,
    collapsed: false,
  },
};

function declareClaude1m(modelId: string, enabled: boolean): string {
  if (!enabled || modelId.trim().toLowerCase().endsWith("[1m]")) return modelId;
  return `${modelId}[1m]`;
}

function defaultTargetStatuses(): TargetLiveStatus[] {
  return [
    {
      kind: "claude_code",
      installed: false,
      version: null,
      configPath: "~/.claude/settings.json",
      status: "not_applied",
      appliedSiteId: null,
      appliedSiteName: null,
      appliedModelId: null,
      providerId: null,
      orphan: false,
      liveSummary: {},
      lastAppliedAt: null,
      staleReason: null,
    },
    {
      kind: "codex",
      installed: false,
      version: null,
      configPath: "~/.codex/config.toml",
      status: "not_applied",
      appliedSiteId: null,
      appliedSiteName: null,
      appliedModelId: null,
      providerId: null,
      orphan: false,
      liveSummary: {},
      lastAppliedAt: null,
      staleReason: null,
    },
    {
      kind: "pi",
      installed: false,
      version: null,
      configPath: "~/.pi/agent/models.json",
      status: "not_applied",
      appliedSiteId: null,
      appliedSiteName: null,
      appliedModelId: null,
      providerId: null,
      orphan: false,
      liveSummary: {},
      lastAppliedAt: null,
      staleReason: null,
    },
    {
      kind: "prime",
      installed: false,
      version: null,
      configPath: "~/.prime/agent/models.json",
      status: "not_applied",
      appliedSiteId: null,
      appliedSiteName: null,
      appliedModelId: null,
      providerId: null,
      orphan: false,
      liveSummary: {},
      lastAppliedAt: null,
      staleReason: null,
    },
    {
      kind: "zcode",
      installed: false,
      version: null,
      configPath: "~/.zcode/v2/config.json",
      status: "not_applied",
      appliedSiteId: null,
      appliedSiteName: null,
      appliedModelId: null,
      providerId: null,
      orphan: false,
      liveSummary: {},
      lastAppliedAt: null,
      staleReason: null,
    },
  ];
}

let settings: AppSettings = { ...DEFAULT_SETTINGS };
let sites: Site[] = [];
/** 浏览器 mock 的代理请求头：与 Rust 侧一样按站点加密存储，列表只回计数。 */
let siteProxyHeaders = new Map<string, ProxyHeader[]>();
/** 浏览器 mock 的代理运行态：只为让页面在无 Tauri 环境下可交互。 */
let proxyRuntime: { running: boolean; startedAt: number | null; total: number; success: number; failed: number; lastError: string | null } | null = null;
let proxyRequests: LocalProxyRequestLogEntry[] = [];
const MOCK_PROXY_PATH_TOKEN = "0123456789abcdef0123456789abcdef";

function mockProxyStatus(): LocalProxyStatus {
  const running = proxyRuntime?.running ?? false;
  const targets = (["claude_code", "codex", "pi", "prime", "zcode"] as TargetKind[]).map((target) => {
    const takeover = settings.localProxyTargets.includes(target);
    const binding = targetStatuses.find((item) => item.kind === target);
    const suffix = target === "codex" || target === "pi" || target === "prime" ? "/v1" : "";
    return {
      target,
      takeover,
      siteId: binding?.appliedSiteId ?? null,
      siteName: binding?.appliedSiteName ?? null,
      clientBaseUrl: `http://127.0.0.1:${settings.localProxyPort}/${MOCK_PROXY_PATH_TOKEN}/t/${target}${suffix}`,
    };
  });
  return {
    running,
    address: `127.0.0.1:${settings.localProxyPort}`,
    port: settings.localProxyPort,
    pathToken: MOCK_PROXY_PATH_TOKEN,
    startedAt: proxyRuntime?.startedAt ?? null,
    uptimeSeconds: proxyRuntime?.startedAt ? Math.floor((Date.now() - proxyRuntime.startedAt) / 1000) : 0,
    totalRequests: proxyRuntime?.total ?? 0,
    successRequests: proxyRuntime?.success ?? 0,
    failedRequests: proxyRuntime?.failed ?? 0,
    activeConnections: 0,
    lastError: proxyRuntime?.lastError ?? null,
    targets,
  };
}
let backups: BackupInfo[] = [];
let targetStatuses: TargetLiveStatus[] = defaultTargetStatuses();
let webdavConfig: WebDavConfigView = {
  baseUrl: "",
  username: "",
  remotePath: "xiaobai-switch",
  acceptInvalidCerts: false,
  hasPassword: false,
  autoSyncEnabled: false,
  syncIntervalMinutes: 60,
  maxRemoteBackups: 3,
};
let remoteBackups: RemoteBackupInfo[] = [];
let localBackups: LocalBackupInfo[] = [];
let latestLocalBackupAt: number | null = null;
let webdavLastAttemptAt: number | null = null;
let webdavLastSuccessAt: number | null = null;
let webdavSyncRevision = 0;
const models = new Map<string, SiteModel[]>();
const thinkingPresets = new Map<string, SiteThinkingPreset>();
const keys = new Map<string, string>();
const keySecrets = new Map<string, string>();
const exclusions = new Map<string, Set<string>>();
let quotaProbeCallCount = 0;
let quotaProbeHandler: ((site: Site) => SiteQuota | Promise<SiteQuota>) | null = null;
let mcpServers: McpServer[] = [];

/** 浏览器模式下模拟「其它客户端里手工配过的 MCP」，覆盖托管/自有/需要密钥三类。 */
const INITIAL_SCANNED_MCP: ScannedMcp[] = [
  {
    target: "codex",
    key: "existing-fs",
    name: "existing-fs",
    managed: false,
    kind: "stdio",
    config: { command: "npx", args: ["-y", "existing-fs-mcp"] },
    envKeys: [],
    headerKeys: [],
    importedId: null,
  },
  {
    target: "claude_code",
    key: "existing-db",
    name: "existing-db",
    managed: false,
    kind: "stdio",
    config: { command: "uvx", args: ["existing-db-mcp"] },
    envKeys: ["DB_URL"],
    headerKeys: [],
    importedId: null,
  },
  {
    target: "codex",
    key: "xiaobai_managed-one",
    name: "managed-one",
    managed: true,
    kind: "stdio",
    config: { command: "npx", args: ["managed-one"] },
    envKeys: [],
    headerKeys: [],
    importedId: null,
  },
  {
    target: "zcode",
    key: "existing-zcode",
    name: "existing-zcode",
    managed: false,
    kind: "stdio",
    config: { command: "npx", args: ["-y", "existing-zcode-mcp"] },
    envKeys: [],
    headerKeys: [],
    importedId: null,
  },
];
let scannedMcp: ScannedMcp[] = INITIAL_SCANNED_MCP.map((entry) => ({ ...entry }));
let agentRules: AgentRules = { body: "", targets: [], updatedAt: 0 };

/** 浏览器模式下全局约束的落点样例，覆盖「已存在」与「尚未创建」两种状态。 */
const AGENT_RULES_PATHS: [TargetKind, string, boolean][] = [
  ["claude_code", "/Users/demo/.claude/CLAUDE.md", true],
  ["codex", "/Users/demo/.codex/AGENTS.md", false],
  ["pi", "/Users/demo/.pi/agent/AGENTS.md", false],
  ["prime", "/Users/demo/.prime/agent/AGENTS.md", false],
  ["zcode", "/Users/demo/.zcode/AGENTS.md", false],
];

type BrowserMarketplaceSkill = Omit<MarketplaceSkill, "installedTargets">;

const SKILL_ROOTS: Record<SkillTarget, string> = {
  agents: "/Users/demo/.agents/skills",
  claude_code: "/Users/demo/.claude/skills",
  codex: "/Users/demo/.codex/skills",
  pi: "/Users/demo/.pi/agent/skills",
  prime: "/Users/demo/.prime/agent/skills",
};

const INITIAL_SKILLS: Skill[] = [
  {
    name: "find-skills",
    description: "Shared agent skills installed under ~/.agents/skills",
    author: "XiaoBaiSwitch Plus",
    version: "1.0.0",
    target: "agents",
    sourcePath: "/Users/demo/.agents/skills/find-skills/SKILL.md",
    directoryPath: "/Users/demo/.agents/skills/find-skills",
    skillsPath: SKILL_ROOTS.agents,
    enabled: true,
  },
  {
    name: "shared-tools",
    description: "Shared workflows installed for Claude Code",
    author: "XiaoBaiSwitch Plus",
    version: "1.0.0",
    target: "claude_code",
    sourcePath: "/Users/demo/.claude/skills/shared-tools/SKILL.md",
    directoryPath: "/Users/demo/.claude/skills/shared-tools",
    skillsPath: SKILL_ROOTS.claude_code,
    enabled: true,
  },
  {
    name: "shared-tools",
    description: "The same skill installed independently for Codex",
    author: "XiaoBaiSwitch Plus",
    version: "1.0.0",
    target: "codex",
    sourcePath: "/Users/demo/.codex/skills/shared-tools/SKILL.md",
    directoryPath: "/Users/demo/.codex/skills/shared-tools",
    skillsPath: SKILL_ROOTS.codex,
    enabled: true,
  },
  {
    name: "pi-workflow",
    description: "Example Pi workflow skill",
    author: "XiaoBaiSwitch Plus",
    version: "1.1.0",
    target: "pi",
    sourcePath: "/Users/demo/.pi/agent/skills/pi-workflow/SKILL.md",
    directoryPath: "/Users/demo/.pi/agent/skills/pi-workflow",
    skillsPath: SKILL_ROOTS.pi,
    enabled: true,
  },
  {
    name: "prime-workflow",
    description: "Example Prime workflow skill",
    author: "XiaoBaiSwitch Plus",
    version: "1.2.0",
    target: "prime",
    sourcePath: "/Users/demo/.prime/agent/skills/prime-workflow/SKILL.md",
    directoryPath: "/Users/demo/.prime/agent/skills/prime-workflow",
    skillsPath: SKILL_ROOTS.prime,
    enabled: true,
  },
];

const MARKETPLACE_SKILLS: BrowserMarketplaceSkill[] = [
  {
    name: "shared-tools",
    description: "Reusable coding-agent workflows",
    repo: "demo/shared-tools",
    stars: 128,
    installs: 2048,
  },
  {
    name: "design-system",
    description: "Build and review design systems",
    repo: "demo/design-system",
    stars: 96,
    installs: 1024,
  },
];

let skills: Skill[] = INITIAL_SKILLS.map((skill) => ({ ...skill }));
let marketplaceTargets = new Map<string, SkillTarget[]>([
  ["demo/shared-tools", ["claude_code", "codex"]],
]);
let installedSkillSources = initialInstalledSkillSources();

function skillSourceKey(target: SkillTarget, sourcePath: string): string {
  return `${target}:${sourcePath}`;
}

function initialInstalledSkillSources(): Map<string, string> {
  return new Map([
    [`claude_code:${SKILL_ROOTS.claude_code}/shared-tools/SKILL.md`, "demo/shared-tools"],
    [`codex:${SKILL_ROOTS.codex}/shared-tools/SKILL.md`, "demo/shared-tools"],
  ]);
}

export function resetBrowserMock() {
  settings = { ...DEFAULT_SETTINGS };
  sites = [];
  siteProxyHeaders = new Map();
  proxyRuntime = null;
  proxyRequests = [];
  backups = [];
  targetStatuses = defaultTargetStatuses();
  webdavConfig = {
    baseUrl: "",
    username: "",
    remotePath: "xiaobai-switch",
    acceptInvalidCerts: false,
    hasPassword: false,
    autoSyncEnabled: false,
    syncIntervalMinutes: 60,
    maxRemoteBackups: 3,
  };
  remoteBackups = [];
  localBackups = [];
  latestLocalBackupAt = null;
  webdavLastAttemptAt = null;
  webdavLastSuccessAt = null;
  models.clear();
  thinkingPresets.clear();
  keys.clear();
  keySecrets.clear();
  exclusions.clear();
  quotaProbeCallCount = 0;
  quotaProbeHandler = null;
  mcpServers = [];
  scannedMcp = INITIAL_SCANNED_MCP.map((entry) => ({ ...entry }));
  agentRules = { body: "", targets: [], updatedAt: 0 };
  skills = INITIAL_SKILLS.map((skill) => ({ ...skill }));
  marketplaceTargets = new Map([
    ["demo/shared-tools", ["claude_code", "codex"]],
  ]);
  installedSkillSources = initialInstalledSkillSources();
}

export function getBrowserQuotaProbeCallCount() {
  return quotaProbeCallCount;
}

export function setBrowserQuotaProbeHandler(
  handler: ((site: Site) => SiteQuota | Promise<SiteQuota>) | null,
) {
  quotaProbeHandler = handler;
}

export function seedTargetStatuses(items: TargetLiveStatus[]) {
  targetStatuses = items;
}

export function seedBackups(items: BackupInfo[]) {
  backups = items;
}

export function seedWebDavMock(
  config: Partial<WebDavConfigView>,
  items: RemoteBackupInfo[] = [],
) {
  webdavConfig = { ...webdavConfig, ...config };
  remoteBackups = items;
}

export function seedLocalBackups(items: LocalBackupInfo[]) {
  localBackups = items;
  latestLocalBackupAt = items[0]?.createdAt ?? null;
}

function now() {
  return Date.now();
}

function uid() {
  return crypto.randomUUID();
}

function makeApiKey(
  _siteId: string,
  secret: string,
  label: string,
  isActive: boolean,
): SiteApiKeySummary {
  const id = uid();
  keySecrets.set(id, secret);
  return {
    id,
    label,
    keyPrefix: keyPrefix(secret),
    isActive,
    quotaRevision: uid(),
    selectedModelId: null,
    lastModelFetchAt: null,
    lastModelFetchLatencyMs: null,
    lastModelFetchError: null,
  };
}

function syncSiteApiKeys(site: Site, incoming: NonNullable<UpdateSiteInput["apiKeys"]>): Site {
  if (incoming.length === 0) {
    throw { code: "validation_failed", message: "API key is required" };
  }
  const current = site.apiKeys ?? [];
  const next: SiteApiKeySummary[] = [];
  const kept = new Set<string>();
  for (const item of incoming) {
    const secret = item.apiKey.trim();
    if (!secret) throw { code: "validation_failed", message: "API key is required" };
    const existing = item.id ? current.find((key) => key.id === item.id) : undefined;
    if (item.id && !existing) {
      throw { code: "validation_failed", message: "api key does not belong to this site" };
    }
    if (existing) {
      const previous = keySecrets.get(existing.id);
      const changed = previous !== secret;
      if (changed) keySecrets.set(existing.id, secret);
      next.push({
        ...existing,
        label: item.label?.trim() || existing.label,
        keyPrefix: changed ? keyPrefix(secret) : existing.keyPrefix,
        quotaRevision: changed ? uid() : existing.quotaRevision,
        isActive: false,
      });
      kept.add(existing.id);
    } else {
      const created = makeApiKey(
        site.id,
        secret,
        item.label?.trim() || `K ${next.length + 1}`,
        false,
      );
      next.push(created);
      kept.add(created.id);
    }
  }
  for (const old of current) {
    if (!kept.has(old.id)) {
      keySecrets.delete(old.id);
      models.delete(old.id);
      exclusions.delete(old.id);
    }
  }
  if (next[0]) next[0] = { ...next[0], isActive: true };
  return projectSite({ ...site, apiKeys: next });
}

function projectSite(site: Site): Site {
  const apiKeys = site.apiKeys ?? [];
  const active = apiKeys.find((key) => key.isActive) ?? apiKeys[0] ?? null;
  if (!active) {
    return { ...site, apiKeys, activeApiKeyId: null, hasKey: false };
  }
  keys.set(site.id, keySecrets.get(active.id) ?? "");
  return {
    ...site,
    apiKeys,
    activeApiKeyId: active.id,
    keyPrefix: active.keyPrefix,
    quotaRevision: active.quotaRevision,
    selectedModelId: active.selectedModelId,
    lastModelFetchAt: active.lastModelFetchAt,
    lastModelFetchLatencyMs: active.lastModelFetchLatencyMs,
    lastModelFetchError: active.lastModelFetchError,
    hasKey: true,
  };
}

function updateSiteKeys(siteId: string, apiKeys: SiteApiKeySummary[]): Site {
  const current = sites.find((s) => s.id === siteId);
  if (!current) throw { code: "not_found", message: "Site not found" };
  const next = projectSite({ ...current, apiKeys, updatedAt: now() });
  sites = sites.map((s) => (s.id === siteId ? next : s));
  return next;
}

function requireKey(site: Site, apiKeyId?: string | null): SiteApiKeySummary {
  const keysForSite = site.apiKeys ?? [];
  const id = apiKeyId || site.activeApiKeyId;
  const key = keysForSite.find((item) => item.id === id) ?? keysForSite.find((item) => item.isActive);
  if (!key) throw { code: "not_found", message: "api key not found" };
  if (apiKeyId && !key.isActive) {
    throw { code: "validation_failed", message: "api key is not the site's current key" };
  }
  return key;
}

/** Mirrors the Rust is_opencode_go_base gate: https + opencode.ai + `/zen/go` segments. */
function isOpencodeGoBase(baseUrl: string): boolean {
  try {
    const url = new URL(baseUrl.trim());
    if (url.protocol !== "https:") return false;
    if (url.hostname !== "opencode.ai") return false;
    const segments = url.pathname.split("/").filter(Boolean);
    return segments.some(
      (segment, index) => segment === "zen" && segments[index + 1] === "go",
    );
  } catch {
    return false;
  }
}

function normalizeSkillSource(source: string): string {
  const clean = source.trim().replace(/\/$/, "").replace(/\.git$/, "");
  const githubMarker = "github.com/";
  const githubIndex = clean.toLowerCase().indexOf(githubMarker);
  return (githubIndex >= 0 ? clean.slice(githubIndex + githubMarker.length) : clean)
    .toLowerCase();
}

function skillNameFromSource(source: string): string {
  const normalized = normalizeSkillSource(source);
  const parts = normalized.split("/").filter(Boolean);
  const name = parts[parts.length - 1];
  if (!name) throw { code: "validation_failed", message: "Skill source is required" };
  return name;
}

export async function handleBrowserCommand<T>(
  cmd: string,
  args?: Record<string, unknown>,
): Promise<T> {
  switch (cmd) {
    case "get_settings":
      return settings as T;
    case "save_settings": {
      const partial = (args?.partial ?? {}) as Partial<AppSettings>;
      settings = { ...settings, ...partial };
      if (!settings.closeToTray) settings.startInTray = false;
      return settings as T;
    }
    case "restore_main_window":
    case "force_quit":
    case "refresh_tray_menu":
      return undefined as T;
    case "list_skills":
      return skills.map((skill) => ({ ...skill })) as T;
    case "get_skill": {
      const target = args?.target as SkillTarget;
      const sourcePath = String(args?.sourcePath ?? "");
      const skill = skills.find(
        (item) => item.target === target && item.sourcePath === sourcePath,
      );
      if (!skill) throw { code: "not_found", message: "Skill not found" };
      return {
        info: { ...skill },
        content: `# ${skill.name}\n\n${skill.description}`,
        files: ["SKILL.md"],
      } as T;
    }
    case "set_skill_enabled": {
      const target = args?.target as SkillTarget;
      const sourcePath = String(args?.sourcePath ?? "");
      const index = skills.findIndex(
        (skill) => skill.target === target && skill.sourcePath === sourcePath,
      );
      if (index < 0) throw { code: "not_found", message: "Skill not found" };
      const enabled = Boolean(args?.enabled);
      const nextSourcePath = `${skills[index].directoryPath}/${
        enabled ? "SKILL.md" : "SKILL.md.disabled"
      }`;
      skills = skills.map((skill, skillIndex) =>
        skillIndex === index
          ? {
              ...skill,
              enabled,
              sourcePath: nextSourcePath,
            }
          : skill,
      );
      const sourceRef = installedSkillSources.get(skillSourceKey(target, sourcePath));
      if (sourceRef) {
        installedSkillSources.delete(skillSourceKey(target, sourcePath));
        installedSkillSources.set(skillSourceKey(target, nextSourcePath), sourceRef);
      }
      return undefined as T;
    }
    case "install_skill": {
      const source = String(args?.source ?? "").trim();
      const target = args?.target as SkillTarget;
      const skillsPath = SKILL_ROOTS[target];
      if (!skillsPath) throw { code: "validation_failed", message: "Invalid skill target" };
      const requestedName = String(args?.skillName ?? "").trim();
      const name = requestedName || skillNameFromSource(source);
      const directoryPath = `${skillsPath}/${name}`;
      const installed: Skill = {
        name,
        description: `Installed from ${source}`,
        author: null,
        version: null,
        target,
        sourcePath: `${directoryPath}/SKILL.md`,
        directoryPath,
        skillsPath,
        enabled: true,
      };
      const previous = skills.find(
        (skill) => skill.target === target && skill.directoryPath === directoryPath,
      );
      const previousSourceRef = previous
        ? installedSkillSources.get(skillSourceKey(target, previous.sourcePath))
        : undefined;
      if (previous && previousSourceRef) {
        installedSkillSources.delete(skillSourceKey(target, previous.sourcePath));
        marketplaceTargets.set(
          previousSourceRef,
          (marketplaceTargets.get(previousSourceRef) ?? []).filter(
            (value) => value !== target,
          ),
        );
      }
      skills = [
        ...skills.filter(
          (skill) => !(skill.target === target && skill.directoryPath === directoryPath),
        ),
        installed,
      ];
      const sourceRef = normalizeSkillSource(source);
      installedSkillSources.set(skillSourceKey(target, installed.sourcePath), sourceRef);
      const installedTargets = marketplaceTargets.get(sourceRef) ?? [];
      marketplaceTargets.set(sourceRef, [...new Set([...installedTargets, target])]);
      if (requestedName) {
        const skillKey = `${sourceRef}::${requestedName.toLowerCase()}`;
        const namedTargets = marketplaceTargets.get(skillKey) ?? [];
        marketplaceTargets.set(skillKey, [...new Set([...namedTargets, target])]);
      }
      return name as T;
    }
    case "uninstall_skill": {
      const target = args?.target as SkillTarget;
      const sourcePath = String(args?.sourcePath ?? "");
      const skill = skills.find(
        (item) => item.target === target && item.sourcePath === sourcePath,
      );
      if (!skill) return undefined as T;
      skills = skills.filter(
        (item) => !(item.target === target && item.sourcePath === sourcePath),
      );
      const sourceKey = skillSourceKey(target, sourcePath);
      const sourceRef = installedSkillSources.get(sourceKey);
      installedSkillSources.delete(sourceKey);
      const sourceStillInstalled = sourceRef && skills.some(
        (item) =>
          item.target === target &&
          installedSkillSources.get(skillSourceKey(item.target, item.sourcePath)) === sourceRef,
      );
      if (sourceRef && !sourceStillInstalled) {
        marketplaceTargets.set(
          sourceRef,
          (marketplaceTargets.get(sourceRef) ?? []).filter((value) => value !== target),
        );
      }
      return undefined as T;
    }
    case "search_skill_marketplace": {
      const query = String(args?.query ?? "").trim().toLowerCase();
      const source = String(args?.source ?? "skills.sh");
      if (source !== "skills.sh" && source !== "github") {
        throw { code: "validation_failed", message: "Unsupported skill marketplace source" };
      }
      return MARKETPLACE_SKILLS.filter((skill) =>
        [skill.name, skill.description, skill.repo].some((value) =>
          value.toLowerCase().includes(query),
        ),
      ).map((skill) => {
        const repo = normalizeSkillSource(skill.repo);
        const named = marketplaceTargets.get(`${repo}::${skill.name.toLowerCase()}`);
        const repoTargets = marketplaceTargets.get(repo) ?? [];
        const installedTargets = source === "github"
          ? [...repoTargets]
          : [...(named ?? repoTargets)];
        return {
          ...skill,
          stars: source === "github" ? skill.stars : 0,
          installs: source === "skills.sh" ? skill.installs : 0,
          installedTargets,
        };
      }) as T;
    }
    case "list_sites":
      return sites as T;
    case "get_site": {
      const site = sites.find((s) => s.id === args?.id);
      if (!site) throw { code: "not_found", message: "Site not found" };
      return site as T;
    }
    case "get_site_api_key": {
      const id = args?.id as string;
      const site = sites.find((item) => item.id === id);
      if (!site) throw { code: "not_found", message: "Site not found" };
      const keys = site.apiKeys ?? [];
      const wanted = args?.apiKeyId as string | undefined;
      const key = wanted
        ? keys.find((item) => item.id === wanted)
        : (keys.find((item) => item.isActive) ?? keys[0]);
      if (!key) throw { code: "not_found", message: "api key not found" };
      return (keySecrets.get(key.id) ?? "") as T;
    }
    case "create_site": {
      const input = args?.input as CreateSiteInput;
      const id = uid();
      const t = now();
      const urls =
        input.baseUrls && input.baseUrls.length > 0
          ? input.baseUrls
          : [input.baseUrl ?? ""];
      const firstLabel = input.apiKeyLabel?.trim() || "K 1";
      const apiKeys = [makeApiKey(id, input.apiKey, firstLabel, true)];
      for (const extra of input.extraApiKeys ?? []) {
        const secret = extra.apiKey?.trim() ?? "";
        if (!secret) throw { code: "validation_failed", message: "API key is required" };
        if (apiKeys.some((item) => keySecrets.get(item.id) === secret)) {
          throw { code: "validation_failed", message: "this API key already exists on the site" };
        }
        const label = extra.label?.trim() || `K ${apiKeys.length + 1}`;
        if (apiKeys.some((item) => item.label.toLowerCase() === label.toLowerCase())) {
          throw { code: "validation_failed", message: "key name already exists on this site" };
        }
        apiKeys.push(makeApiKey(id, secret, label, false));
      }
      const active = apiKeys[0]!;
      const site = projectSite({
        id,
        name: input.name,
        baseUrl: urls[0] ?? "",
        baseUrls: urls,
        keyPrefix: active.keyPrefix,
        quotaRevision: active.quotaRevision,
        hasKey: true,
        protocol: input.protocol ?? "openai_compatible",
        claudeAuthKeyStyle: input.claudeAuthKeyStyle ?? "anthropic_auth_token",
        notes: input.notes ?? null,
        enabled: true,
        sortOrder: sites.length,
        selectedModelId: null,
        lastModelFetchAt: null,
        lastModelFetchLatencyMs: null,
        lastModelFetchError: null,
        createdAt: t,
        updatedAt: t,
        capabilities: input.capabilities ?? {},
        activeApiKeyId: active.id,
        apiKeys,
        zcodeApiType: input.zcodeApiType || null,
      });
      if (input.proxyHeaders) {
        siteProxyHeaders.set(site.id, input.proxyHeaders);
        site.proxyHeaderCount = input.proxyHeaders.length;
      }
      sites = [...sites, site];
      return site as T;
    }
    case "import_site_from_deep_link": {
      const input = args?.input as DeepLinkSiteImportInput;
      const name = input?.name?.trim() ?? "";
      const apiKey = input?.apiKey?.trim() ?? "";
      if (!name) throw { code: "validation_failed", message: "site name is required" };
      if (!apiKey) throw { code: "validation_failed", message: "API key is required" };
      const protocol = input.protocol === "anthropic" ? "anthropic" : "openai_compatible";
      const urls =
        input.baseUrls && input.baseUrls.length > 0
          ? input.baseUrls
          : [];
      if (urls.length === 0) {
        throw { code: "validation_failed", message: "at least one base URL is required" };
      }
      const sameSet = (a: string[], b: string[]) => {
        if (a.length !== b.length) return false;
        const sa = [...a].sort();
        const sb = [...b].sort();
        return sa.every((v, i) => v === sb[i]);
      };
      const existing = sites.find(
        (s) => s.protocol === protocol && sameSet(s.baseUrls ?? [s.baseUrl], urls),
      );
      if (existing) {
        const currentKeys = existing.apiKeys ?? [];
        const matched = currentKeys.find((item) => keySecrets.get(item.id) === apiKey);
        let apiKeys = currentKeys;
        let addedApiKey = false;
        if (!matched) {
          const label = input.keyName?.trim() || `K ${currentKeys.length + 1}`;
          apiKeys = [...currentKeys, makeApiKey(existing.id, apiKey, label, false)];
          addedApiKey = true;
        }
        const updated = projectSite({
          ...existing,
          name,
          notes: input.notes !== undefined ? input.notes : existing.notes,
          capabilities:
            input.capabilities !== undefined ? input.capabilities : existing.capabilities,
          apiKeys,
          updatedAt: now(),
        });
        sites = sites.map((s) => (s.id === existing.id ? updated : s));
        const result: DeepLinkSiteImportResult = {
          site: updated,
          created: false,
          addedApiKey,
          reusedApiKey: Boolean(matched),
          activatedApiKey: false,
        };
        return result as T;
      }
      const created = await handleBrowserCommand<Site>("create_site", {
        input: {
          name,
          baseUrls: urls,
          baseUrl: urls[0],
          apiKey,
          protocol,
          notes: input.notes ?? null,
          capabilities: input.capabilities,
        } satisfies CreateSiteInput,
      });
      const result: DeepLinkSiteImportResult = {
        site: created,
        created: true,
        addedApiKey: true,
        reusedApiKey: false,
        activatedApiKey: true,
      };
      return result as T;
    }
    case "update_site": {
      const id = args?.id as string;
      const input = (args?.input ?? {}) as UpdateSiteInput;
      sites = sites.map((s) => {
        if (s.id !== id) return s;
        if (input.apiKeys) {
          s = syncSiteApiKeys(s, input.apiKeys);
        } else if (input.apiKey) {
          const active = (s.apiKeys ?? []).find((item) => item.isActive);
          if (active) {
            keySecrets.set(active.id, input.apiKey);
            active.keyPrefix = keyPrefix(input.apiKey);
            active.quotaRevision = uid();
          }
        }
        let baseUrls = s.baseUrls?.length ? s.baseUrls : [s.baseUrl];
        let baseUrl = s.baseUrl;
        if (input.baseUrls && input.baseUrls.length > 0) {
          baseUrls = input.baseUrls;
          baseUrl = baseUrls[0];
        } else if (input.baseUrl) {
          if (baseUrls.includes(input.baseUrl)) {
            baseUrls = [input.baseUrl, ...baseUrls.filter((u) => u !== input.baseUrl)];
          } else {
            baseUrls = [input.baseUrl, ...baseUrls.slice(1)];
          }
          baseUrl = input.baseUrl;
        }
        return {
          ...s,
          name: input.name ?? s.name,
          baseUrl,
          baseUrls,
          keyPrefix: input.apiKey ? keyPrefix(input.apiKey) : s.keyPrefix,
          quotaRevision: input.apiKey
            ? ((s.apiKeys ?? []).find((item) => item.isActive)?.quotaRevision ?? uid())
            : s.quotaRevision,
          protocol: input.protocol ?? s.protocol,
          claudeAuthKeyStyle: input.claudeAuthKeyStyle ?? s.claudeAuthKeyStyle,
          notes: input.notes !== undefined ? input.notes : s.notes,
          enabled: input.enabled ?? s.enabled,
          selectedModelId:
            input.selectedModelId !== undefined ? input.selectedModelId : s.selectedModelId,
          sortOrder: input.sortOrder ?? s.sortOrder,
          capabilities: input.capabilities !== undefined ? input.capabilities : s.capabilities,
          zcodeApiType:
            input.zcodeApiType !== undefined
              ? input.zcodeApiType || null
              : (s.zcodeApiType ?? null),
          updatedAt: now(),
        };
      });
      const found = sites.find((s) => s.id === id);
      if (!found) throw { code: "not_found", message: "Site not found" };
      if (input.proxyHeaders) {
        siteProxyHeaders.set(id, input.proxyHeaders);
        found.proxyHeaderCount = input.proxyHeaders.length;
      }
      const site = projectSite(found);
      sites = sites.map((s) => (s.id === id ? site : s));
      return site as T;
    }
    case "delete_site": {
      const id = args?.id as string;
      const site = sites.find((s) => s.id === id);
      for (const key of site?.apiKeys ?? []) {
        models.delete(key.id);
        keySecrets.delete(key.id);
        exclusions.delete(key.id);
      }
      sites = sites.filter((s) => s.id !== id);
      keys.delete(id);
      for (const key of [...thinkingPresets.keys()]) {
        if (key.startsWith(`${id}:`)) thinkingPresets.delete(key);
      }
      return undefined as T;
    }
    case "reorder_sites": {
      const ids = args?.ids as string[];
      sites = ids
        .map((id, i) => {
          const s = sites.find((x) => x.id === id);
          return s ? { ...s, sortOrder: i } : null;
        })
        .filter(Boolean) as Site[];
      return undefined as T;
    }
    case "fetch_site_models": {
      const siteId = args?.siteId as string;
      const site = sites.find((s) => s.id === siteId);
      if (!site) throw { code: "not_found", message: "Site not found" };
      const key = requireKey(site, args?.apiKeyId as string | undefined);
      const sample: SiteModel[] = [
        {
          id: uid(),
          siteId,
          apiKeyId: key.id,
          modelId: "gpt-4.1",
          displayName: "gpt-4.1",
          ownedBy: "mock",
          raw: null,
          isManual: false,
        },
        {
          id: uid(),
          siteId,
          apiKeyId: key.id,
          modelId: "claude-sonnet-4",
          displayName: "claude-sonnet-4",
          ownedBy: "mock",
          raw: null,
          isManual: false,
        },
      ];
      const existing = models.get(key.id) ?? [];
      const hidden = exclusions.get(key.id) ?? new Set<string>();
      const visibleSample = sample.filter((m) => !hidden.has(m.modelId));
      const fetchedIds = new Set(visibleSample.map((m) => m.modelId));
      const manuals = existing.filter((m) => m.isManual && !fetchedIds.has(m.modelId) && !hidden.has(m.modelId));
      const merged = [...visibleSample, ...manuals];
      models.set(key.id, merged);
      const fetchedAt = now();
      const apiKeys = (site.apiKeys ?? []).map((item) =>
        item.id === key.id
          ? {
              ...item,
              lastModelFetchAt: fetchedAt,
              lastModelFetchLatencyMs: 42,
              lastModelFetchError: null,
              selectedModelId:
                item.selectedModelId && merged.some((m) => m.modelId === item.selectedModelId)
                  ? item.selectedModelId
                  : (merged[0]?.modelId ?? null),
            }
          : item,
      );
      const next = updateSiteKeys(siteId, apiKeys);
      const result: FetchModelsResult = {
        models: merged,
        latencyMs: 42,
        endpoint: `${next.baseUrl}/v1/models`,
        fetchedAt,
        apiKeyId: key.id,
      };
      return result as T;
    }
    case "list_site_models": {
      const siteId = args?.siteId as string;
      const site = sites.find((s) => s.id === siteId);
      if (!site) return [] as T;
      const key = requireKey(site, args?.apiKeyId as string | undefined);
      return (models.get(key.id) ?? []) as T;
    }
    case "set_selected_model": {
      const siteId = args?.siteId as string;
      const modelId = args?.modelId as string;
      const site = sites.find((s) => s.id === siteId);
      if (!site) throw { code: "not_found", message: "Site not found" };
      const key = requireKey(site);
      exclusions.get(key.id)?.delete(modelId);
      const list = models.get(key.id) ?? [];
      if (!list.some((m) => m.modelId === modelId)) {
        models.set(key.id, [
          ...list,
          {
            id: uid(),
            siteId,
            apiKeyId: key.id,
            modelId,
            displayName: modelId,
            ownedBy: null,
            raw: null,
            isManual: true,
          },
        ]);
      }
      const apiKeys = (site.apiKeys ?? []).map((item) =>
        item.id === key.id ? { ...item, selectedModelId: modelId } : item,
      );
      updateSiteKeys(siteId, apiKeys);
      return undefined as T;
    }
    case "clear_site_models": {
      const siteId = args?.siteId as string;
      const site = sites.find((s) => s.id === siteId);
      if (!site) throw { code: "not_found", message: "Site not found" };
      const key = requireKey(site);
      models.set(key.id, []);
      const apiKeys = (site.apiKeys ?? []).map((item) =>
        item.id === key.id ? { ...item, selectedModelId: null } : item,
      );
      return updateSiteKeys(siteId, apiKeys) as T;
    }
    case "delete_site_model": {
      const siteId = args?.siteId as string;
      const modelId = args?.modelId as string;
      const site = sites.find((s) => s.id === siteId);
      if (!site) throw { code: "not_found", message: "Site not found" };
      const key = requireKey(site);
      const list = (models.get(key.id) ?? []).filter((m) => m.modelId !== modelId);
      if (list.length === (models.get(key.id) ?? []).length) {
        throw { code: "not_found", message: "model not found" };
      }
      const hidden = exclusions.get(key.id) ?? new Set<string>();
      hidden.add(modelId);
      exclusions.set(key.id, hidden);
      models.set(key.id, list);
      const nextSelected =
        key.selectedModelId === modelId ? (list[0]?.modelId ?? null) : key.selectedModelId;
      const apiKeys = (site.apiKeys ?? []).map((item) =>
        item.id === key.id ? { ...item, selectedModelId: nextSelected } : item,
      );
      return updateSiteKeys(siteId, apiKeys) as T;
    }
    case "get_site_thinking_preset": {
      const siteId = String(args?.siteId ?? "");
      const target = (args?.target as "pi" | "prime") ?? "pi";
      return (
        thinkingPresets.get(`${siteId}:${target}`) ?? {
          siteId,
          target,
          defaultLevel: null,
          extended: {},
          models: {},
        }
      ) as T;
    }
    case "save_site_thinking_preset": {
      const preset = args?.preset as SiteThinkingPreset;
      thinkingPresets.set(`${preset.siteId}:${preset.target}`, preset);
      return preset as T;
    }
    case "list_target_status": {
      return targetStatuses as T;
    }
    case "apply_site": {
      const siteId = args?.siteId as string;
      const modelId = args?.modelId as string;
      const site = sites.find((s) => s.id === siteId);
      const targets = ((args?.targets as string[]) ?? []) as TargetKind[];
      const appliedAt = now();
      const claudeUse1mContext = Boolean(args?.claudeUse1mContext);
      const claudeLiveSummary: Record<string, string | null> = {
        model: declareClaude1m(modelId, claudeUse1mContext),
      };
      for (const [argKey, envKey, supports1m] of [
        ["claudeOpusModelId", "ANTHROPIC_DEFAULT_OPUS_MODEL", true],
        ["claudeSonnetModelId", "ANTHROPIC_DEFAULT_SONNET_MODEL", true],
        ["claudeHaikuModelId", "ANTHROPIC_DEFAULT_HAIKU_MODEL", false],
        ["claudeFableModelId", "ANTHROPIC_DEFAULT_FABLE_MODEL", false],
      ] as const) {
        const aliasModel = args?.[argKey];
        if (typeof aliasModel === "string" && aliasModel.trim()) {
          claudeLiveSummary[envKey] = declareClaude1m(
            aliasModel,
            supports1m && claudeUse1mContext,
          );
        }
      }
      const authKey = args?.claudeAuthKeyStyle === "anthropic_api_key"
        ? "ANTHROPIC_API_KEY"
        : "ANTHROPIC_AUTH_TOKEN";
      claudeLiveSummary[authKey] = site?.keyPrefix ?? "sk-xx";
      if (typeof args?.claudeEffortLevel === "string") {
        claudeLiveSummary.effortLevel = args.claudeEffortLevel;
      }
      const piWriteAllModels = Boolean(args?.piWriteAllModels);
      const piModelCount = piWriteAllModels ? Math.max(models.get(siteId)?.length ?? 0, 1) : 1;
      const primeWriteAllModels = Boolean(args?.primeWriteAllModels);
      const primeModelCount = primeWriteAllModels
        ? Math.max(models.get(siteId)?.length ?? 0, 1)
        : 1;
      const zcodeWriteAllModels = Boolean(args?.zcodeWriteAllModels);
      const zcodeModelCount = zcodeWriteAllModels
        ? Math.max(models.get(siteId)?.length ?? 0, 1)
        : 1;
      const zcodeApiType =
        site?.zcodeApiType ?? (site?.protocol === "anthropic" ? "anthropic-messages" : "openai-responses");
      targetStatuses = targetStatuses.map((row) =>
        targets.includes(row.kind)
          ? {
              ...row,
              status: "applied",
              appliedSiteId: siteId,
              appliedSiteName: site?.name ?? null,
              appliedModelId: modelId,
              providerId:
                row.kind === "pi" || row.kind === "prime" || row.kind === "zcode"
                  ? `xiaobai_${siteId.slice(0, 8)}`
                  : row.providerId,
              liveSummary:
                row.kind === "pi"
                  ? {
                      defaultProvider: `xiaobai_${siteId.slice(0, 8)}`,
                      defaultModel: modelId,
                      modelCount: String(piModelCount),
                      writeAllModels: String(piWriteAllModels),
                    }
                  : row.kind === "prime"
                    ? {
                        defaultProvider: `xiaobai_${siteId.slice(0, 8)}`,
                        defaultModel: modelId,
                        modelCount: String(primeModelCount),
                        writeAllModels: String(primeWriteAllModels),
                      }
                    : row.kind === "zcode"
                      ? {
                          defaultProvider: `xiaobai_${siteId.slice(0, 8)}`,
                          defaultModel: modelId,
                          apiType: zcodeApiType,
                          modelCount: String(zcodeModelCount),
                          writeAllModels: String(zcodeWriteAllModels),
                        }
                      : row.kind === "claude_code"
                        ? claudeLiveSummary
                        : row.liveSummary,
              lastAppliedAt: appliedAt,
            }
          : row,
      );
      const result: ApplyResult = {
        siteId,
        modelId,
        results: targets.map((t) => ({
          target: t,
          ok: true,
          status: "applied",
          backupPaths: [],
          message: "Browser mock: apply simulated (no filesystem writes)",
        })),
        appliedAt,
      };
      return result as T;
    }
    case "revert_target":
    case "restore_official_target": {
      const target = args?.target as TargetKind;
      targetStatuses = targetStatuses.map((row) =>
        row.kind === target
          ? {
              ...row,
              status: "not_applied",
              appliedSiteId: null,
              appliedSiteName: null,
              appliedModelId: null,
              lastAppliedAt: null,
              liveSummary: {},
              orphan: false,
              staleReason: null,
            }
          : row,
      );
      return undefined as T;
    }
    case "cleanup_orphan_target":
      return undefined as T;
    case "list_apply_records":
      return [] as T;
    case "list_backups": {
      const target = args?.target as TargetKind | undefined;
      const list = target ? backups.filter((b) => b.target === target) : backups;
      return list as T;
    }
    case "preview_backup": {
      const id = String(args?.id ?? "");
      const b = backups.find((x) => x.id === id);
      const preview: BackupPreview = {
        id,
        summary: {
          ANTHROPIC_MODEL: b?.modelId ?? "gpt-5.6",
          ANTHROPIC_BASE_URL: "https://api.example.com",
        },
        files: (b?.files ?? []).map((name) => ({
          name,
          path: `${b?.dir ?? ""}/${name}`,
        })),
      };
      return preview as T;
    }
    case "delete_backup": {
      const id = String(args?.id ?? "");
      backups = backups.filter((b) => b.id !== id);
      return undefined as T;
    }
    case "restore_backup":
      return undefined as T;
    case "get_webdav_config":
      return webdavConfig as T;
    case "save_webdav_config": {
      const input = args?.input as {
        baseUrl: string;
        username: string;
        password?: string | null;
        remotePath: string;
        acceptInvalidCerts: boolean;
        autoSyncEnabled: boolean;
        syncIntervalMinutes: number;
        maxRemoteBackups: number;
      };
      webdavConfig = {
        baseUrl: input.baseUrl,
        username: input.username,
        remotePath: input.remotePath,
        acceptInvalidCerts: input.acceptInvalidCerts,
        hasPassword: webdavConfig.hasPassword || Boolean(input.password),
        autoSyncEnabled: input.autoSyncEnabled,
        syncIntervalMinutes: input.syncIntervalMinutes,
        maxRemoteBackups: input.maxRemoteBackups,
      };
      return webdavConfig as T;
    }
    case "test_webdav_connection":
      return undefined as T;
    case "create_app_backup": {
      const destination = String(args?.destination ?? "");
      const createdAt = now();
      const fileName = `xiaobai-switch-backup-20260827_120000.browser.12345678.zip`;
      if (destination === "local") {
        latestLocalBackupAt = createdAt;
        localBackups = [
          {
            fileName,
            size: 1024,
            createdAt,
            deviceName: "browser",
            reason: "manual",
            appVersion: "0.0.5",
            error: null,
          },
          ...localBackups,
        ];
      }
      if (destination === "webdav") {
        if (!webdavConfig.baseUrl) {
          throw { code: "webdav_not_configured", message: "WebDAV is not configured" };
        }
        webdavLastAttemptAt = createdAt;
        webdavLastSuccessAt = createdAt;
        remoteBackups = [
          {
            fileName,
            size: 1024,
            lastModified: new Date(createdAt).toUTCString(),
            deviceName: "browser",
          },
          ...remoteBackups,
        ];
      }
      const result: BackupOperationResult = {
        fileName,
        localPath:
          destination === "local" ? `~/.xiaobai-switch/backups/app/${fileName}` : null,
        uploaded: destination === "webdav",
        warning: null,
      };
      return result as T;
    }
    case "sync_now": {
      if (!webdavConfig.baseUrl) {
        throw { code: "webdav_not_configured", message: "WebDAV is not configured" };
      }
      const createdAt = now();
      webdavLastAttemptAt = createdAt;
      webdavLastSuccessAt = createdAt;
      webdavSyncRevision += 1;
      const outcome: SyncOutcome = {
        action: "upload",
        revision: webdavSyncRevision,
        bundleFileName: "xiaobai-switch-backup-20260827_120000.browser.12345678.zip",
        conflict: false,
        pendingRestart: false,
        warning: null,
      };
      return outcome as T;
    }
    case "get_backup_overview": {
      const overview: BackupOverview = {
        latestLocalBackupAt,
        webdavConfigured: Boolean(webdavConfig.baseUrl),
        webdavAutoSyncEnabled: webdavConfig.autoSyncEnabled,
        webdavSync: {
          lastAttemptAt: webdavLastAttemptAt,
          lastSuccessAt: webdavLastSuccessAt,
          status: webdavLastSuccessAt ? "success" : "never",
          error: null,
        },
        syncRevision: webdavSyncRevision > 0 ? webdavSyncRevision : null,
        nextScheduledAt:
          webdavConfig.autoSyncEnabled && webdavLastAttemptAt
            ? webdavLastAttemptAt + webdavConfig.syncIntervalMinutes * 60_000
            : null,
      };
      return overview as T;
    }
    case "list_local_backups":
      return localBackups as T;
    case "delete_local_backup":
      localBackups = localBackups.filter((backup) => backup.fileName !== args?.fileName);
      latestLocalBackupAt = localBackups[0]?.createdAt ?? null;
      return undefined as T;
    case "restore_local_backup":
      return undefined as T;
    case "list_webdav_backups":
      return remoteBackups as T;
    case "delete_webdav_backup":
      remoteBackups = remoteBackups.filter((backup) => backup.fileName !== args?.fileName);
      return undefined as T;
    case "restore_webdav_backup":
      return undefined as T;
    case "take_restore_result":
      return null as T;
    case "detect_cli_tools": {
      const tools: CliToolInfo[] = [
        { kind: "claude_code", installed: false, version: null, path: null },
        { kind: "codex", installed: false, version: null, path: null },
        { kind: "pi", installed: false, version: null, path: null },
        { kind: "prime", installed: false, version: null, path: null },
        { kind: "zcode", installed: false, version: null, path: null },
      ];
      return tools as T;
    }
    // 悬浮窗：浏览器模式返回样例余额，覆盖「正常 / 低余额 / 无限额 / 未知」四种展示。
    case "get_all_sites_quota":
    case "refresh_sites_quota":
      return [
        {
          siteId: "s1",
          siteName: "Relay A",
          enabled: true,
          sortOrder: 0,
          quota: {
            status: "available",
            remainingUsd: 42.5,
            usedUsd: 7.5,
            totalUsd: 50,
            unlimited: false,
            unit: "usd",
            expiresAt: null,
            source: "token_usage",
            endpoint: null,
            fetchedAt: 1,
            latencyMs: 12,
            error: null,
            windows: [],
          },
        },
        {
          siteId: "s2",
          siteName: "Relay B",
          enabled: true,
          sortOrder: 1,
          quota: {
            status: "available",
            remainingUsd: 1.25,
            usedUsd: 8.75,
            totalUsd: 10,
            unlimited: false,
            unit: "usd",
            expiresAt: null,
            source: "token_usage",
            endpoint: null,
            fetchedAt: 1,
            latencyMs: 12,
            error: null,
            windows: [],
          },
        },
        {
          siteId: "s3",
          siteName: "Unlimited C",
          enabled: true,
          sortOrder: 2,
          quota: {
            status: "available",
            remainingUsd: null,
            usedUsd: null,
            totalUsd: null,
            unlimited: true,
            unit: null,
            expiresAt: null,
            source: null,
            endpoint: null,
            fetchedAt: 1,
            latencyMs: 12,
            error: null,
            windows: [],
          },
        },
        {
          siteId: "s4",
          siteName: "Unknown D",
          enabled: false,
          sortOrder: 3,
          quota: null,
        },
        {
          // 刷新失败的站点：界面应当在余额旁显示可查看的原因。
          siteId: "s5",
          siteName: "Failed E",
          enabled: true,
          sortOrder: 4,
          quota: {
            status: "error",
            remainingUsd: null,
            usedUsd: null,
            totalUsd: null,
            unlimited: false,
            unit: null,
            expiresAt: null,
            source: null,
            endpoint: null,
            fetchedAt: 1,
            latencyMs: 0,
            error: "unauthorized: invalid api key",
            windows: [],
          },
        },
      ] as T;
    case "set_floating_window_collapsed": {
      const collapsed = Boolean(args?.collapsed);
      settings = {
        ...settings,
        floatingWindow: {
          enabled: settings.floatingWindow?.enabled ?? true,
          autoRefreshMinutes: settings.floatingWindow?.autoRefreshMinutes ?? 5,
          positionX: settings.floatingWindow?.positionX ?? 100,
          positionY: settings.floatingWindow?.positionY ?? 100,
          collapsed,
        },
      };
      return undefined as T;
    }
    case "toggle_floating_window":
    case "show_floating_window_cmd":
    case "hide_floating_window_cmd":
    case "save_floating_window_position":
    case "reset_floating_window_position":
    case "set_floating_window_enabled":
    case "set_floating_window_refresh_interval":
      return undefined as T;
    case "get_app_paths": {
      const paths: AppPaths = {
        appDir: "~/.xiaobai-switch",
        dbPath: "~/.xiaobai-switch/xiaobai-switch.db",
        masterKeyPath: "~/.xiaobai-switch/master.key",
        backupsDir: "~/.xiaobai-switch/backups",
        appBackupsDir: "~/.xiaobai-switch/backups/app",
        codexEnvPath: "~/.xiaobai-switch/env/codex.env",
        logsDir: "~/.xiaobai-switch/logs",
      };
      return paths as T;
    }
    case "sync_windows_chrome":
    case "set_always_on_top":
    case "minimize_window":
    case "toggle_maximize_window":
    case "open_path":
    case "open_url":
      return undefined as T;
    case "take_pending_deep_link":
      return null as T;
    // 浏览器里没有写 pending-deeplink.url 的平台分支（真实实现 = cfg!(target_os = "macos")）。
    case "deep_link_requires_polling":
      return false as T;
    case "fetch_http_text": {
      const url = String(args?.url ?? "");
      return {
        status: 0,
        contentType: "",
        finalUrl: url,
        body: "",
      } as T;
    }
    case "fetch_http_bytes": {
      const url = String(args?.url ?? "");
      const empty: HttpBytesResult = {
        status: 0,
        contentType: "",
        finalUrl: url,
        base64: "",
      };
      return empty as T;
    }
    case "resolve_http_proxy":
      return null as T;
    case "check_app_update":
      return null as T;
    case "probe_urls": {
      const urls = (args?.urls as string[]) ?? [];
      const results: UrlProbeResult[] = urls.map((url, i) => ({
        url,
        ok: true,
        latencyMs: [80, 1500, 4000][i % 3] ?? 80,
        status: 200,
        error: null,
      }));
      return results as T;
    }
    case "test_newapi_access": {
      const input = (args?.input ?? {}) as {
        accessToken?: string | null;
        userId?: string | null;
      };
      if (!input.userId || !input.accessToken) {
        return {
          ok: false,
          status: 401,
          remainingUsd: null,
          usedUsd: null,
          totalUsd: null,
          endpoint: "",
          message: "Unauthorized, invalid access token",
        } as T;
      }
      return {
        ok: true,
        status: 200,
        remainingUsd: 12.5,
        usedUsd: 3.5,
        totalUsd: 16,
        endpoint: "https://example.invalid/api/user/self",
        message: null,
      } as T;
    }
    case "probe_site_quota": {
      quotaProbeCallCount += 1;
      const siteId = String(args?.siteId ?? "");
      const site = sites.find((s) => s.id === siteId);
      if (!site) throw { code: "not_found", message: "Site not found" };
      if (quotaProbeHandler) return (await quotaProbeHandler(site)) as T;
      if (isOpencodeGoBase(site.baseUrl)) {
        const fetchedAt = now();
        const base: SiteQuota = {
          status: "available",
          remainingUsd: null,
          usedUsd: null,
          totalUsd: null,
          unlimited: false,
          unit: "USD",
          expiresAt: null,
          source: "opencode_go",
          endpoint: "https://opencode.ai/zen/go/v1/usage",
          fetchedAt,
          latencyMs: 9,
          error: null,
        };
        if (!site.hasKey) {
          const unauthorized: SiteQuota = {
            ...base,
            status: "unauthorized",
            source: null,
            endpoint: null,
          };
          return unauthorized as T;
        }
        const result: SiteQuota = {
          ...base,
          windows: [
            {
              kind: "rolling",
              usagePercent: 12.5,
              resetAt: fetchedAt + 2 * 3600_000,
              limitUsd: 12,
            },
            {
              kind: "weekly",
              usagePercent: 46.2,
              resetAt: fetchedAt + 3 * 24 * 3600_000,
              limitUsd: 30,
            },
            {
              kind: "monthly",
              usagePercent: 8.4,
              resetAt: fetchedAt + 20 * 24 * 3600_000,
              limitUsd: 60,
            },
          ],
        };
        return result as T;
      }
      if (!site.hasKey || /no-quota/i.test(site.baseUrl) || /no-quota/i.test(site.name)) {
        const unsupported: SiteQuota = {
          status: "unsupported",
          remainingUsd: null,
          usedUsd: null,
          totalUsd: null,
          unlimited: false,
          expiresAt: null,
          source: null,
          endpoint: null,
          fetchedAt: now(),
          latencyMs: 4,
          error: null,
        };
        return unsupported as T;
      }
      const result: SiteQuota = {
        status: "available",
        remainingUsd: 87.5,
        usedUsd: 12.5,
        totalUsd: 100,
        unlimited: false,
        unit: "USD",
        expiresAt: Math.floor(Date.UTC(2026, 11, 31) / 1000),
        source: "credit_grants",
        endpoint: `${normalizeBaseUrl(site.baseUrl).codexBaseUrl}/dashboard/billing/credit_grants`,
        fetchedAt: now(),
        latencyMs: 12,
        error: null,
      };
      return result as T;
    }
    case "probe_site_api_key": {
      const siteId = args?.siteId as string;
      const apiKey = String(args?.apiKey ?? "").trim();
      const site = sites.find((s) => s.id === siteId);
      if (!site) throw { code: "not_found", message: "Site not found" };
      if (!apiKey) throw { code: "validation_failed", message: "API key is required" };
      if (/fail/i.test(apiKey)) {
        throw { code: "unauthorized", message: "unauthorized" };
      }
      const result: ProbeSiteApiKeyResult = {
        modelCount: 2,
        latencyMs: 12,
        endpoint: `${site.baseUrl}/v1/models`,
      };
      return result as T;
    }
    case "probe_site_model": {
      const siteId = args?.siteId as string;
      const modelId = String(args?.modelId ?? "").trim();
      const site = sites.find((s) => s.id === siteId);
      if (!site) throw { code: "not_found", message: "Site not found" };
      if (!modelId) throw { code: "validation_failed", message: "model id required" };
      await new Promise((r) => setTimeout(r, 5));
      const endpoint = `${normalizeBaseUrl(site.baseUrl).codexBaseUrl}/chat/completions`;
      if (/fail-long/i.test(modelId)) {
        const result: ModelProbeResult = {
          modelId,
          ok: false,
          latencyMs: 8,
          status: 400,
          error:
            "mock upstream rejected this model because the requested identifier is not available on this gateway and the provider returned a very long diagnostic payload",
          endpoint,
        };
        return result as T;
      }
      if (/fail/i.test(modelId)) {
        const result: ModelProbeResult = {
          modelId,
          ok: false,
          latencyMs: 8,
          status: 400,
          error: "mock upstream rejected this model",
          endpoint,
        };
        return result as T;
      }
      const result: ModelProbeResult = {
        modelId,
        ok: true,
        latencyMs: 12,
        status: 200,
        error: null,
        endpoint,
      };
      return result as T;
    }
    case "add_site_api_key": {
      const siteId = args?.siteId as string;
      const input = (args?.input ?? {}) as AddSiteApiKeyInput;
      const site = sites.find((s) => s.id === siteId);
      if (!site) throw { code: "not_found", message: "Site not found" };
      const current = site.apiKeys ?? [];
      if (current.some((item) => keySecrets.get(item.id) === input.apiKey)) {
        throw { code: "validation_failed", message: "this API key already exists on the site" };
      }
      const label = input.label?.trim() || `K ${current.length + 1}`;
      if (current.some((item) => item.label.toLowerCase() === label.toLowerCase())) {
        throw { code: "validation_failed", message: "key name already exists on this site" };
      }
      return updateSiteKeys(siteId, [...current, makeApiKey(siteId, input.apiKey, label, false)]) as T;
    }
    case "update_site_api_key": {
      const siteId = args?.siteId as string;
      const apiKeyId = args?.apiKeyId as string;
      const input = (args?.input ?? {}) as UpdateSiteApiKeyInput;
      const site = sites.find((s) => s.id === siteId);
      if (!site) throw { code: "not_found", message: "Site not found" };
      const current = site.apiKeys ?? [];
      if (!current.some((item) => item.id === apiKeyId)) {
        throw { code: "not_found", message: "api key not found" };
      }
      const apiKeys = current.map((item) => {
        if (item.id !== apiKeyId) return item;
        if (input.apiKey) keySecrets.set(item.id, input.apiKey);
        return {
          ...item,
          label: input.label?.trim() || item.label,
          keyPrefix: input.apiKey ? keyPrefix(input.apiKey) : item.keyPrefix,
          quotaRevision: input.apiKey ? uid() : item.quotaRevision,
        };
      });
      return updateSiteKeys(siteId, apiKeys) as T;
    }
    case "delete_site_api_key": {
      const siteId = args?.siteId as string;
      const apiKeyId = args?.apiKeyId as string;
      const site = sites.find((s) => s.id === siteId);
      if (!site) throw { code: "not_found", message: "Site not found" };
      const current = site.apiKeys ?? [];
      const target = current.find((item) => item.id === apiKeyId);
      if (!target) throw { code: "not_found", message: "api key not found" };
      if (current.length <= 1) {
        throw { code: "validation_failed", message: "the last API key cannot be deleted" };
      }
      if (target.isActive) {
        throw { code: "validation_failed", message: "switch to another API key before deleting the current one" };
      }
      models.delete(apiKeyId);
      keySecrets.delete(apiKeyId);
      exclusions.delete(apiKeyId);
      return updateSiteKeys(
        siteId,
        current.filter((item) => item.id !== apiKeyId),
      ) as T;
    }
    case "switch_site_api_key": {
      const siteId = args?.siteId as string;
      const apiKeyId = args?.apiKeyId as string;
      const site = sites.find((s) => s.id === siteId);
      if (!site) throw { code: "not_found", message: "Site not found" };
      const current = site.apiKeys ?? [];
      if (!current.some((item) => item.id === apiKeyId)) {
        throw { code: "not_found", message: "api key not found" };
      }
      const apiKeys = current.map((item) => ({ ...item, isActive: item.id === apiKeyId }));
      const switched = updateSiteKeys(siteId, apiKeys);
      const fetched = await handleBrowserCommand<FetchModelsResult>("fetch_site_models", {
        siteId,
        apiKeyId,
      });
      const result: SwitchSiteApiKeyResult = {
        site: sites.find((s) => s.id === siteId) ?? switched,
        models: fetched.models,
        fetch: {
          ok: true,
          apiKeyId,
          latencyMs: fetched.latencyMs,
          endpoint: fetched.endpoint,
          fetchedAt: fetched.fetchedAt,
          error: null,
        },
        results: [],
      };
      return result as T;
    }
    case "switch_site_route": {
      const siteId = args?.siteId as string;
      const baseUrl = String(args?.baseUrl ?? "").trim();
      const current = sites.find((s) => s.id === siteId);
      if (!current) throw { code: "not_found", message: "Site not found" };
      const urls = current.baseUrls?.length ? current.baseUrls : [current.baseUrl];
      if (!urls.includes(baseUrl)) {
        throw { code: "validation_failed", message: "base URL is not a configured route" };
      }
      const next = [baseUrl, ...urls.filter((u) => u !== baseUrl)];
      sites = sites.map((s) =>
        s.id === siteId ? { ...s, baseUrl, baseUrls: next, updatedAt: now() } : s,
      );
      const site = sites.find((s) => s.id === siteId)!;
      const result: SwitchRouteResult = { site, results: [] };
      return result as T;
    }
    case "list_mcp_servers":
      return mcpServers.map(({ config: _config, env: _env, headers: _headers, ...summary }) => ({
        ...summary,
      })) as T;
    case "get_mcp_server": {
      const id = String(args?.id ?? "");
      const server = mcpServers.find((item) => item.id === id);
      if (!server) throw { code: "not_found", message: "MCP server not found" };
      return { ...server } as T;
    }
    case "save_mcp_server": {
      const input = (args?.input ?? {}) as McpServerInput;
      const name = (input.name ?? "").trim();
      if (!name) throw { code: "validation_failed", message: "MCP server name is required" };
      if (!/^[A-Za-z0-9_-]+$/.test(name)) {
        throw {
          code: "validation_failed",
          message: "MCP server name may only contain letters, numbers, '_' and '-'",
        };
      }
      for (const [field, value] of Object.entries({
        config: input.config,
        env: input.env,
        headers: input.headers,
      })) {
        if (value !== undefined && (typeof value !== "object" || value === null || Array.isArray(value))) {
          throw { code: "validation_failed", message: `MCP ${field} must be a JSON object` };
        }
      }
      if (
        mcpServers.some(
          (item) => item.name.toLowerCase() === name.toLowerCase() && item.id !== input.id,
        )
      ) {
        throw { code: "validation_failed", message: "another MCP server is already named" };
      }
      const timestamp = now();
      const saved: McpServer = {
        id: input.id ?? uid(),
        name,
        kind: input.kind ?? "stdio",
        enabled: input.enabled ?? false,
        targets: input.targets ?? [],
        config: input.config ?? {},
        env: input.env ?? {},
        headers: input.headers ?? {},
        createdAt: mcpServers.find((item) => item.id === input.id)?.createdAt ?? timestamp,
        updatedAt: timestamp,
      };
      mcpServers = mcpServers.map((item) => (item.id === saved.id ? saved : item));
      if (!mcpServers.some((item) => item.id === saved.id)) mcpServers.push(saved);
      return { server: saved, sweep: { results: [], appliedAt: timestamp } } as T;
    }
    case "delete_mcp_server": {
      const id = String(args?.id ?? "");
      if (!mcpServers.some((item) => item.id === id)) {
        throw { code: "not_found", message: "MCP server not found" };
      }
      mcpServers = mcpServers.filter((item) => item.id !== id);
      return { results: [], appliedAt: now() } as T;
    }
    case "local_proxy_status":
      return mockProxyStatus() as T;

    case "start_local_proxy": {
      proxyRuntime = { running: true, startedAt: Date.now(), total: 0, success: 0, failed: 0, lastError: null };
      return mockProxyStatus() as T;
    }

    case "stop_local_proxy": {
      if (proxyRuntime) proxyRuntime.running = false;
      // 与后端一致：停止代理会把接管目标改回直连。
      settings.localProxyTargets = [];
      return mockProxyStatus() as T;
    }

    case "set_local_proxy_takeover": {
      const target = args?.target as TargetKind;
      const enabled = Boolean(args?.enabled);
      if (enabled && !(proxyRuntime?.running ?? false)) {
        throw { code: "proxy_not_running", message: "start the local proxy first" };
      }
      const next = new Set(settings.localProxyTargets);
      if (enabled) next.add(target);
      else next.delete(target);
      settings.localProxyTargets = [...next];
      return mockProxyStatus() as T;
    }

    case "set_local_proxy_port": {
      const port = Number(args?.port ?? 0);
      if (!Number.isFinite(port) || port < 1024 || port > 65535) {
        throw { code: "validation_failed", message: "port must be between 1024 and 65535" };
      }
      settings.localProxyPort = Math.round(port);
      return mockProxyStatus() as T;
    }

    case "list_local_proxy_requests": {
      const limit = (args?.limit as number | undefined) ?? 100;
      return proxyRequests.slice(0, limit) as T;
    }

    case "clear_local_proxy_requests": {
      proxyRequests = [];
      return undefined as T;
    }

    case "get_site_proxy_headers": {
      const siteId = args?.siteId as string;
      return (siteProxyHeaders.get(siteId) ?? []) as T;
    }

    case "validate_site_proxy_headers":
      return undefined as T;

    case "apply_mcp_servers": {
      const targets = (args?.targets ?? []) as TargetKind[];
      const results: McpApplyTargetResult[] = targets.map((target) => ({
        target,
        ok: true,
        backupPaths: [],
        message: `Applied to ${target} (browser mock)`,
      }));
      const result: McpApplyResult = { results, appliedAt: now() };
      return result as T;
    }
    case "scan_existing_mcp": {
      // 与已保存的 MCP 比对，标出已纳管项（真实实现同样由后端比对）。
      const entries = scannedMcp.map((entry) => {
        const existing = mcpServers.find(
          (server) => server.name.toLowerCase() === entry.name.toLowerCase(),
        );
        return { ...entry, importedId: existing?.id ?? null };
      });
      const outcome: ScanOutcome = { entries, warnings: [] };
      return outcome as T;
    }
    case "import_scanned_mcp": {
      const locators = (args?.locators ?? []) as McpImportLocator[];
      const result: McpImportResult = { imported: [], failed: [] };
      for (const locator of locators) {
        const entry = scannedMcp.find(
          (item) => item.target === locator.target && item.key === locator.key,
        );
        if (!entry) {
          result.failed.push({ ...locator, message: "entry not found" });
          continue;
        }
        const timestamp = now();
        const saved: McpServer = {
          id: uid(),
          name: entry.name,
          kind: entry.kind,
          enabled: true,
          targets: [],
          config: entry.config,
          env: {},
          headers: {},
          createdAt: timestamp,
          updatedAt: timestamp,
        };
        mcpServers.push(saved);
        const { config: _config, env: _env, headers: _headers, ...summary } = saved;
        result.imported.push(summary);
      }
      return result as T;
    }
    case "mcp_target_paths":
      return [
        ["claude_code", "/Users/demo/.claude.json"],
        ["codex", "/Users/demo/.codex/config.toml"],
        ["pi", "/Users/demo/.pi/agent/mcp.json"],
        ["prime", "/Users/demo/.prime/agent/settings.json"],
        ["zcode", "/Users/demo/.zcode/cli/config.json"],
      ] as T;
    case "get_agent_rules":
      return { ...agentRules, targets: [...agentRules.targets] } as T;
    case "save_agent_rules": {
      const body = String(args?.body ?? "");
      const targets = (args?.targets ?? []) as TargetKind[];
      for (const marker of [
        "<!-- xiaobai-switch:begin global-rules -->",
        "<!-- xiaobai-switch:end global-rules -->",
      ]) {
        if (body.includes(marker)) {
          throw {
            code: "validation_failed",
            message: "the rules text must not contain the XiaoBaiSwitch managed-block markers",
          };
        }
      }
      const timestamp = now();
      agentRules = { body, targets, updatedAt: timestamp };
      // 正文为空时不写文件：与后端的「清空即清理」语义一致。
      const effective = body.trim().length === 0 ? [] : targets;
      const results = effective.map((target) => ({
        target,
        ok: true,
        path: AGENT_RULES_PATHS.find(([kind]) => kind === target)?.[1] ?? "",
        changed: true,
        backupPaths: [],
        message: "applied",
      }));
      const result: AgentRulesApplyResult = { results, appliedAt: timestamp };
      return result as T;
    }
    case "agent_rules_target_paths":
      return AGENT_RULES_PATHS.map(([target, path, exists]) => ({
        target,
        path,
        exists,
      })) as T;
    case "search_mcp_registry":
    case "discover_mcp_registry": {
      // 浏览器模式下的固定样例，覆盖「本地包」「远程服务」「无法安装」三类，
      // 让 UI 的三条渲染分支都能在开发时看到。真实数据来自官方仓库。
      const all: RegistryCandidate[] = [
        {
          name: "io.github.example/filesystem",
          description: "Local filesystem access (browser mock)",
          version: "1.2.3",
          repositoryUrl: "https://github.com/example/fs",
          installKinds: ["package"],
          draft: {
            name: "filesystem",
            displayName: "io.github.example/filesystem",
            kind: "stdio",
            config: {
              command: "npx",
              args: ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"],
            },
            env: { MODE: "readonly" },
            headers: {},
            requiredFields: [
              { name: "API_KEY", description: "Access token", secret: true, kind: "env" },
            ],
            repositoryUrl: "https://github.com/example/fs",
          },
        },
        {
          name: "io.github.example/no-config-needed",
          description: "Needs no user input (browser mock)",
          version: "2.0.0",
          repositoryUrl: null,
          installKinds: ["package"],
          draft: {
            name: "no-config-needed",
            displayName: "io.github.example/no-config-needed",
            kind: "stdio",
            config: { command: "uvx", args: ["no-config-needed"] },
            env: {},
            headers: {},
            // 没有必填项：一键安装应当直接装好，不弹表单。
            requiredFields: [],
            repositoryUrl: null,
          },
        },
        {
          name: "ai.example/hosted-memory",
          description: "Hosted memory service (browser mock)",
          version: "0.4.0",
          repositoryUrl: null,
          installKinds: ["remote"],
          draft: {
            name: "hosted-memory",
            displayName: "ai.example/hosted-memory",
            kind: "http",
            config: { url: "https://mcp.example.ai" },
            env: {},
            headers: {},
            requiredFields: [
              {
                name: "Authorization",
                description: "Bearer token",
                secret: true,
                kind: "header",
              },
            ],
            repositoryUrl: null,
          },
        },
        {
          name: "io.example/not-installable",
          description: "No packages or remotes (browser mock)",
          version: "0.0.1",
          repositoryUrl: null,
          installKinds: [],
          draft: null,
        },
      ];
      const term = String(args?.query ?? "").trim().toLowerCase();
      let candidates = term
        ? all.filter((item) => item.name.toLowerCase().includes(term))
        : all;
      if (args?.localOnly === true) {
        candidates = candidates.filter(
          (item) => item.draft?.kind === "stdio" && Boolean(item.draft.config.command),
        );
      }
      return { candidates, nextCursor: null } as T;
    }
    case "preview_urls":
      return normalizeBaseUrl(String(args?.baseUrl ?? "")) as T;
    default:
      throw {
        code: "internal",
        message: `Unknown command in browser mock: ${cmd}`,
      };
  }
}
