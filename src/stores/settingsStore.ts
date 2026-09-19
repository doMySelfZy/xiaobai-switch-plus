import { create } from "zustand";
import { invoke } from "@/lib/invoke";
import type { AppSettings } from "@/types/domain";

const DEFAULT: AppSettings = {
  language: "zh-CN",
  themeMode: "system",
  primaryColor: "#1677ff",
  autoStart: false,
  alwaysOnTop: false,
  claudeHomeOverride: null,
  codexHomeOverride: null,
  piAgentDirOverride: null,
  primeAgentDirOverride: null,
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
};

interface SettingsState {
  settings: AppSettings;
  loaded: boolean;
  loading: boolean;
  fetchSettings: () => Promise<void>;
  saveSettings: (partial: Partial<AppSettings>) => Promise<void>;
}

export const useSettingsStore = create<SettingsState>((set) => ({
  settings: DEFAULT,
  loaded: false,
  loading: false,
  fetchSettings: async () => {
    set({ loading: true });
    try {
      const settings = await invoke<AppSettings>("get_settings");
      set({ settings, loaded: true });
    } finally {
      set({ loading: false });
    }
  },
  saveSettings: async (partial) => {
    const settings = await invoke<AppSettings>("save_settings", { partial });
    set({ settings });
  },
}));
