import { create } from "zustand";
import { invoke } from "@/lib/invoke";
import type {
  McpApplyResult,
  McpImportLocator,
  McpImportResult,
  McpSaveResult,
  McpServer,
  McpServerInput,
  McpServerSummary,
  McpTargetDrift,
  RegistrySearchResult,
  ScanOutcome,
} from "@/types/mcp";
import type { TargetKind } from "@/types/domain";

interface McpState {
  servers: McpServerSummary[];
  loading: boolean;
  /** 各目标客户端相对 DB 期望态的漂移计划（不含 Prime）。 */
  drift: McpTargetDrift[];
  loadServers: () => Promise<void>;
  /** 只读比对各客户端配置与 DB 期望态，刷新漂移标记。 */
  loadDrift: () => Promise<void>;
  getServer: (id: string) => Promise<McpServer>;
  saveServer: (input: McpServerInput) => Promise<McpSaveResult>;
  deleteServer: (id: string) => Promise<McpApplyResult>;
  applyServers: (targets: TargetKind[]) => Promise<McpApplyResult>;
  searchRegistry: (
    query: string,
    options?: { cursor?: string | null; localOnly?: boolean; minResults?: number },
  ) => Promise<RegistrySearchResult>;
  discoverRegistry: (options?: {
    localOnly?: boolean;
    minResults?: number;
  }) => Promise<RegistrySearchResult>;
  /** 扫描四个客户端里用户已有的 MCP（只读，不含密钥值）。 */
  scanExisting: () => Promise<ScanOutcome>;
  /** 纳管：把扫描到的条目导入数据库，密钥由后端读盘取得。 */
  importScanned: (locators: McpImportLocator[]) => Promise<McpImportResult>;
}

export const useMcpStore = create<McpState>((set) => ({
  servers: [],
  loading: false,
  drift: [],

  loadServers: async () => {
    set({ loading: true });
    try {
      const servers = await invoke<McpServerSummary[]>("list_mcp_servers");
      set({ servers, loading: false });
    } catch (error) {
      console.error("Failed to load MCP servers:", error);
      set({ loading: false });
    }
  },

  loadDrift: async () => {
    try {
      const drift = await invoke<McpTargetDrift[]>("mcp_drift_status");
      set({ drift });
    } catch (error) {
      console.error("Failed to load MCP drift status:", error);
      set({ drift: [] });
    }
  },

  getServer: async (id: string) => invoke<McpServer>("get_mcp_server", { id }),

  saveServer: async (input: McpServerInput) => {
    const result = await invoke<McpSaveResult>("save_mcp_server", { input });
    const servers = await invoke<McpServerSummary[]>("list_mcp_servers");
    set({ servers });
    return result;
  },

  deleteServer: async (id: string) => {
    const result = await invoke<McpApplyResult>("delete_mcp_server", { id });
    const servers = await invoke<McpServerSummary[]>("list_mcp_servers");
    set({ servers });
    return result;
  },

  applyServers: async (targets: TargetKind[]) =>
    invoke<McpApplyResult>("apply_mcp_servers", { targets }),

  searchRegistry: async (query, options) =>
    invoke<RegistrySearchResult>("search_mcp_registry", {
      query,
      cursor: options?.cursor ?? null,
      localOnly: options?.localOnly ?? true,
      minResults: options?.minResults ?? 0,
    }),

  // 首屏「热门」：后端用一批常见类目词并发查仓库再合并，比只拉「最近更新」更有用。
  discoverRegistry: async (options) =>
    invoke<RegistrySearchResult>("discover_mcp_registry", {
      localOnly: options?.localOnly ?? true,
      minResults: options?.minResults ?? 20,
    }),

  scanExisting: async () => invoke<ScanOutcome>("scan_existing_mcp"),

  importScanned: async (locators) => {
    const result = await invoke<McpImportResult>("import_scanned_mcp", { locators });
    // 纳管会改变列表，重新拉一次保持一致。
    const servers = await invoke<McpServerSummary[]>("list_mcp_servers");
    set({ servers });
    return result;
  },
}));
