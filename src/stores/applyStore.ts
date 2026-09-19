import { create } from "zustand";
import { invoke } from "@/lib/invoke";
import type {
  ApplyRecord,
  ApplyRequest,
  ApplyResult,
  BackupInfo,
  CliToolInfo,
  TargetKind,
  TargetLiveStatus,
} from "@/types/domain";

interface ApplyState {
  statuses: TargetLiveStatus[];
  tools: CliToolInfo[];
  records: ApplyRecord[];
  backups: BackupInfo[];
  applying: boolean;
  /** True while a foreground status load is in flight. */
  loading: boolean;
  /** True after at least one successful status load (enables cache reuse). */
  statusHydrated: boolean;
  lastResult: ApplyResult | null;
  loadStatus: (opts?: { force?: boolean; background?: boolean }) => Promise<void>;
  detectTools: (opts?: { force?: boolean }) => Promise<void>;
  /** Load status (and derived tools) once; subsequent calls refresh in background. */
  ensureApplyData: () => Promise<void>;
  apply: (req: ApplyRequest) => Promise<ApplyResult>;
  revert: (target: TargetKind) => Promise<void>;
  restoreOfficial: (target: TargetKind) => Promise<void>;
  cleanupOrphan: (target: TargetKind) => Promise<void>;
  loadRecords: () => Promise<void>;
  loadBackups: (target?: TargetKind) => Promise<void>;
  deleteBackup: (id: string) => Promise<void>;
  restoreBackup: (id: string) => Promise<void>;
}

function toolsFromStatuses(statuses: TargetLiveStatus[]): CliToolInfo[] {
  return statuses.map((s) => ({
    kind: s.kind,
    installed: s.installed,
    version: s.version,
    path: null,
  }));
}

export const useApplyStore = create<ApplyState>((set, get) => ({
  statuses: [],
  tools: [],
  records: [],
  backups: [],
  applying: false,
  loading: false,
  statusHydrated: false,
  lastResult: null,
  loadStatus: async (opts) => {
    const background = opts?.background === true;
    if (!background) set({ loading: true });
    try {
      const statuses = await invoke<TargetLiveStatus[]>("list_target_status", {
        force: opts?.force === true,
      });
      set({
        statuses,
        tools: toolsFromStatuses(statuses),
        statusHydrated: true,
      });
    } finally {
      if (!background) set({ loading: false });
    }
  },
  detectTools: async () => {
    // Prefer derived tools from status when available — avoid double CLI probe.
    if (get().statusHydrated && get().tools.length > 0) {
      return;
    }
    const tools = await invoke<CliToolInfo[]>("detect_cli_tools");
    set({ tools });
  },
  ensureApplyData: async () => {
    if (get().statusHydrated && get().statuses.length > 0) {
      // Keep UI snappy: reuse cache and refresh in background.
      void get().loadStatus({ background: true });
      return;
    }
    await get().loadStatus();
  },
  apply: async (req) => {
    set({ applying: true });
    try {
      const result = await invoke<ApplyResult>("apply_site", {
        siteId: req.siteId,
        targets: req.targets,
        modelId: req.modelId,
        claudeAuthKeyStyle: req.claudeAuthKeyStyle,
        claudeOpusModelId: req.claudeOpusModelId ?? null,
        claudeSonnetModelId: req.claudeSonnetModelId ?? null,
        claudeHaikuModelId: req.claudeHaikuModelId ?? null,
        claudeFableModelId: req.claudeFableModelId ?? null,
        claudeEffortLevel: req.claudeEffortLevel ?? null,
        claudeUse1mContext: req.claudeUse1mContext ?? false,
        codexWriteAllModels: req.codexWriteAllModels ?? false,
        codexReasoningEffort: req.codexReasoningEffort ?? null,
        codexRemoteCompaction: req.codexRemoteCompaction ?? false,
        codexImageUnderstanding: req.codexImageUnderstanding ?? false,
        codexImageGeneration: req.codexImageGeneration ?? false,
        codexWebSearch: req.codexWebSearch ?? false,
        codexCapabilitySource: req.codexCapabilitySource ?? "site",
        piWriteAllModels: req.piWriteAllModels ?? false,
        primeWriteAllModels: req.primeWriteAllModels ?? false,
        apiKeyId: req.apiKeyId ?? null,
      });
      set({ lastResult: result });
      await get().loadStatus({ force: true });
      await get().loadBackups();
      return result;
    } finally {
      set({ applying: false });
    }
  },
  revert: async (target) => {
    await invoke("revert_target", { target });
    await get().loadStatus({ force: true });
  },
  restoreOfficial: async (target) => {
    await invoke("restore_official_target", { target });
    await get().loadStatus({ force: true });
    await get().loadBackups();
  },
  cleanupOrphan: async (target) => {
    await invoke("cleanup_orphan_target", { target });
    await get().loadStatus({ force: true });
  },
  loadRecords: async () => {
    const records = await invoke<ApplyRecord[]>("list_apply_records", { limit: 50 });
    set({ records });
  },
  loadBackups: async (target) => {
    const backups = await invoke<BackupInfo[]>(
      "list_backups",
      target ? { target } : {},
    );
    set({ backups });
  },
  deleteBackup: async (id) => {
    await invoke("delete_backup", { id });
    await get().loadBackups();
  },
  restoreBackup: async (id) => {
    await invoke("restore_backup", { id });
    await get().loadStatus({ force: true });
    await get().loadBackups();
  },
}));
