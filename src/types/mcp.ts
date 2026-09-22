import type { TargetKind } from "./domain";

export type McpKind = "stdio" | "sse" | "http";

export interface McpServerSummary {
  id: string;
  name: string;
  kind: McpKind;
  enabled: boolean;
  targets: TargetKind[];
  createdAt: number;
  updatedAt: number;
  currentVersion?: string | null;
  latestVersion?: string | null;
  lastUpdateCheckAt?: number | null;
}

export interface McpServer {
  id: string;
  name: string;
  kind: McpKind;
  enabled: boolean;
  targets: TargetKind[];
  config: Record<string, unknown>;
  env: Record<string, unknown>;
  headers: Record<string, unknown>;
  createdAt: number;
  updatedAt: number;
  currentVersion?: string | null;
  latestVersion?: string | null;
  lastUpdateCheckAt?: number | null;
}

export interface McpServerInput {
  id?: string;
  name: string;
  kind?: McpKind;
  enabled?: boolean;
  targets?: TargetKind[];
  config?: Record<string, unknown>;
  env?: Record<string, unknown>;
  headers?: Record<string, unknown>;
}

export interface McpApplyTargetResult {
  target: TargetKind;
  ok: boolean;
  backupPaths: string[];
  message: string;
}

export interface McpApplyResult {
  results: McpApplyTargetResult[];
  appliedAt: number;
}

export interface McpSaveResult {
  server: McpServer;
  sweep: McpApplyResult;
}

/** 官方 MCP Registry 条目的安装方式。 */
export type RegistryInstallKind = "package" | "remote";

/** 需要用户自己填的字段（仓库标为必填且没有默认值）。 */
export interface RegistryRequiredField {
  name: string;
  description?: string | null;
  /** 敏感字段（仓库标记 isSecret），用密码框并要求用户确认。 */
  secret: boolean;
  /** 决定填到表单的环境变量还是请求头输入框。 */
  kind: "env" | "header";
}

/** 仓库条目换算出的安装草稿：表单初值 + 还需用户补的必填项。 */
export interface RegistryInstallDraft {
  name: string;
  displayName: string;
  kind: McpKind;
  config: Record<string, unknown>;
  env: Record<string, unknown>;
  headers: Record<string, unknown>;
  requiredFields: RegistryRequiredField[];
  repositoryUrl?: string | null;
}

export interface RegistryCandidate {
  name: string;
  description?: string | null;
  version?: string | null;
  repositoryUrl?: string | null;
  installKinds: RegistryInstallKind[];
  /** 无法安装的条目没有草稿，界面应禁用「安装」。 */
  draft?: RegistryInstallDraft | null;
}

export interface RegistrySearchResult {
  candidates: RegistryCandidate[];
  nextCursor?: string | null;
}

/** 扫描目标：四个 Agent 客户端。 */
export type ScanTarget = "claude_code" | "codex" | "pi" | "prime";

/**
 * 扫描到的已有 MCP。
 *
 * **不含任何密钥值**——只带依赖的密钥键名（`envKeys` / `headerKeys`），
 * 界面据此提示「需要填什么」。密钥要到纳管时才由后端直接读盘入库。
 */
export interface ScannedMcp {
  target: ScanTarget;
  /** 客户端配置里的原始键名（可能带 xiaobai_ 前缀）。 */
  key: string;
  /** 归一化名称：与库内记录比对用。 */
  name: string;
  /** 由本工具写入（带托管前缀）；这类不提供纳管。 */
  managed: boolean;
  kind: McpKind;
  /** 启动方式摘要，用于展示「将运行：npx -y xxx」。 */
  config: Record<string, unknown>;
  envKeys: string[];
  headerKeys: string[];
  /** 非空表示已纳管过，界面不该重复提供纳管。 */
  importedId?: string | null;
  /**
   * 同名冲突：库里有同归一化名的记录，但粗身份不同。
   * 导了撞重名校验、应用了撞接管校验，界面直接跳过、不提供纳管。
   */
  nameConflict: boolean;
  /**
   * 等价可接管：按粗身份命中库内记录，该记录启用并覆盖本目标。
   * 下次应用删未托管写托管；只是展示预告，删写仍由后端严格判定。
   */
  adoptable: boolean;
}

export interface ScanWarning {
  target: ScanTarget;
  message: string;
}

export interface ScanOutcome {
  entries: ScannedMcp[];
  warnings: ScanWarning[];
}

export interface McpImportLocator {
  target: ScanTarget;
  key: string;
}

export interface McpImportFailure {
  target: ScanTarget;
  key: string;
  message: string;
}

/**
 * 纳管时命中库内已有行粗身份的条目：只回定位符与已有行的 id/名字，
 * 不含 config/env/headers——不建行、不动已有行。
 */
export interface AlreadyImportedMcp {
  target: ScanTarget;
  key: string;
  existingId: string;
  existingName: string;
}

export interface McpImportResult {
  imported: McpServerSummary[];
  failed: McpImportFailure[];
  /** 命中已有行粗身份、被跳过而未建行的条目（failed 只表示真错误）。 */
  alreadyImported: AlreadyImportedMcp[];
  /**
   * 纳管即接管：入库后立即对涉及客户端写盘接管的结果（删手工条目、写 xiaobai_）。
   * 本次没有任何记录需要接管时为 null/缺省。
   */
  apply?: McpApplyResult | null;
}
