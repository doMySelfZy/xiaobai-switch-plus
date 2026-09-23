import { create } from "zustand";
import type { SiteDeepLinkPayload } from "@/lib/siteDeepLink";

export type AppPage = "sites" | "apply" | "skills" | "mcp" | "rules" | "proxy" | "settings";
export type SettingsSection = "general" | "network" | "paths" | "backup" | "about";
/** Apply center left sidebar target. */
export type ApplyTargetTab = "claude_code" | "codex" | "pi" | "prime";
/** MCP 页左侧 per-agent 侧栏目标（不含 Prime）。 */
export type McpAgentTab = "claude_code" | "codex" | "pi";

interface UIState {
  activePage: AppPage;
  /** Settings sidebar section. */
  settingsTab: SettingsSection;
  /** Apply center target tab. */
  applyTab: ApplyTargetTab;
  /** MCP per-agent 侧栏目标。 */
  mcpTab: McpAgentTab;
  selectedSiteId: string | null;
  /** One-shot site id from “go apply”; apply panels consume then clear. */
  applyPrefillSiteId: string | null;
  /** One-shot add-site form prefill from a deep link without an API key. */
  pendingSiteForm: SiteDeepLinkPayload | null;
  wizardOpen: boolean;
  setPage: (page: AppPage) => void;
  setSettingsTab: (tab: SettingsSection) => void;
  setApplyTab: (tab: ApplyTargetTab) => void;
  setMcpTab: (tab: McpAgentTab) => void;
  setSelectedSiteId: (id: string | null) => void;
  setApplyPrefillSiteId: (id: string | null) => void;
  setPendingSiteForm: (payload: SiteDeepLinkPayload | null) => void;
  setWizardOpen: (open: boolean) => void;
}

export const useUIStore = create<UIState>((set) => ({
  activePage: "sites",
  settingsTab: "general",
  applyTab: "claude_code",
  mcpTab: "claude_code",
  selectedSiteId: null,
  applyPrefillSiteId: null,
  pendingSiteForm: null,
  wizardOpen: false,
  setPage: (page) => set({ activePage: page }),
  setSettingsTab: (settingsTab) => set({ settingsTab }),
  setApplyTab: (applyTab) => set({ applyTab }),
  setMcpTab: (mcpTab) => set({ mcpTab }),
  setSelectedSiteId: (selectedSiteId) => set({ selectedSiteId }),
  setApplyPrefillSiteId: (applyPrefillSiteId) => set({ applyPrefillSiteId }),
  setPendingSiteForm: (pendingSiteForm) => set({ pendingSiteForm }),
  setWizardOpen: (wizardOpen) => set({ wizardOpen }),
}));
