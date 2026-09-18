import { useEffect, useRef, useState } from "react";
import { ConfigProvider, App as AntdApp, theme, Layout } from "antd";
import zhCN from "antd/locale/zh_CN";
import enUS from "antd/locale/en_US";
import { useTranslation } from "react-i18next";
import { TitleBar } from "@/components/layout/TitleBar";
import { SideNav } from "@/components/layout/SideNav";
import { SitesPage } from "@/pages/SitesPage";
import { ApplyPage } from "@/pages/ApplyPage";
import { SkillsPage } from "@/pages/SkillsPage";
import { McpPage } from "@/pages/McpPage";
import { RulesPage } from "@/pages/RulesPage";
import { ProxyPage } from "@/pages/ProxyPage";
import { SettingsPage } from "@/pages/SettingsPage";
import { useSettingsStore, useUIStore, type AppPage } from "@/stores";
import { useResolvedDarkMode } from "@/hooks/useResolvedDarkMode";
import { useSiteDeepLink } from "@/hooks/useSiteDeepLink";
import { useTrayEvents } from "@/hooks/useTrayEvents";
import { useAutoCheckUpdate } from "@/hooks/useUpdateChecker";
import { invoke, isTauri } from "@/lib/invoke";
import { useSiteStore } from "@/stores";
import type { RestoreStartupResult } from "@/types/domain";
import "./i18n";

async function showWindow() {
  try {
    const { getCurrentWebviewWindow } = await import("@tauri-apps/api/webviewWindow");
    const window = getCurrentWebviewWindow();
    await window.show();
    await window.setFocus();
  } catch (e) {
    console.warn("Failed to show window:", e);
  }
}

/**
 * Keep visited main pages mounted (display:none) so return visits are instant.
 * First visit mounts immediately — each page shows its own skeleton while data loads.
 */
function KeepAlivePages({ activePage }: { activePage: AppPage }) {
  const [mounted, setMounted] = useState<Set<AppPage>>(() => new Set([activePage]));

  useEffect(() => {
    setMounted((prev) => {
      if (prev.has(activePage)) return prev;
      const next = new Set(prev);
      next.add(activePage);
      return next;
    });
  }, [activePage]);

  return (
    <div className="relative h-full min-h-0 w-full overflow-hidden">
      {mounted.has("sites") && (
        <div
          className="h-full min-h-0"
          style={{ display: activePage === "sites" ? "flex" : "none" }}
          aria-hidden={activePage !== "sites"}
        >
          <div className="flex h-full min-h-0 w-full flex-col">
            <SitesPage />
          </div>
        </div>
      )}
      {mounted.has("apply") && (
        <div
          className="h-full min-h-0"
          style={{ display: activePage === "apply" ? "flex" : "none" }}
          aria-hidden={activePage !== "apply"}
        >
          <div className="flex h-full min-h-0 w-full flex-col">
            <ApplyPage />
          </div>
        </div>
      )}
      {mounted.has("skills") && (
        <div
          className="h-full min-h-0 overflow-hidden"
          style={{
            display: activePage === "skills" ? "flex" : "none",
            flexDirection: "column",
          }}
          aria-hidden={activePage !== "skills"}
        >
          <div className="flex h-full min-h-0 w-full min-w-0 flex-1 flex-col overflow-hidden">
            <SkillsPage />
          </div>
        </div>
      )}
      {mounted.has("mcp") && (
        <div
          className="h-full min-h-0"
          style={{ display: activePage === "mcp" ? "flex" : "none" }}
          aria-hidden={activePage !== "mcp"}
        >
          <div className="flex h-full min-h-0 w-full flex-col">
            <McpPage />
          </div>
        </div>
      )}
      {mounted.has("rules") && (
        <div
          className="h-full min-h-0"
          style={{ display: activePage === "rules" ? "flex" : "none" }}
          aria-hidden={activePage !== "rules"}
        >
          <div className="flex h-full min-h-0 w-full flex-col">
            <RulesPage />
          </div>
        </div>
      )}
      {mounted.has("proxy") && (
        <div
          className="h-full min-h-0"
          style={{ display: activePage === "proxy" ? "flex" : "none" }}
          aria-hidden={activePage !== "proxy"}
        >
          <div className="flex h-full min-h-0 w-full flex-col">
            <ProxyPage />
          </div>
        </div>
      )}
      {mounted.has("settings") && (
        <div
          className="h-full min-h-0"
          style={{ display: activePage === "settings" ? "flex" : "none" }}
          aria-hidden={activePage !== "settings"}
        >
          <div className="flex h-full min-h-0 w-full flex-col">
            <SettingsPage />
          </div>
        </div>
      )}
    </div>
  );
}

function AppInner({ isDark }: { isDark: boolean }) {
  const { token } = theme.useToken();
  const { modal, message } = AntdApp.useApp();
  const { i18n, t } = useTranslation();
  const activePage = useUIStore((s) => s.activePage);
  const fetchSettings = useSettingsStore((s) => s.fetchSettings);
  const settingsLoaded = useSettingsStore((s) => s.loaded);
  const autoRefreshMinutes = useSettingsStore(
    (s) => s.settings.floatingWindow?.autoRefreshMinutes ?? 5,
  );
  const refreshAllSites = useSiteStore((s) => s.refreshAllSites);
  const rootRef = useRef<HTMLDivElement>(null);
  const restoreResultReadRef = useRef(false);
  useSiteDeepLink({ modal, message });
  useTrayEvents();
  useAutoCheckUpdate();

  useEffect(() => {
    void fetchSettings()
      .catch(() => undefined)
      .then(() => {
        const { language, startInTray } = useSettingsStore.getState().settings;
        if (language) void i18n.changeLanguage(language);
        if (isTauri() && !startInTray) void showWindow();
      });
  }, [fetchSettings, i18n]);

  useEffect(() => {
    if (!settingsLoaded) return;
    const intervalMs = Math.max(1, autoRefreshMinutes) * 60_000;
    void refreshAllSites().catch(() => undefined);
    const timer = window.setInterval(() => {
      void refreshAllSites().catch(() => undefined);
    }, intervalMs);
    return () => window.clearInterval(timer);
  }, [autoRefreshMinutes, refreshAllSites, settingsLoaded]);

  useEffect(() => {
    if (!isTauri()) return;
    let unlisten: (() => void) | undefined;
    let disposed = false;
    void import("@tauri-apps/api/event")
      .then(({ listen }) => listen("sites-refresh-requested", () => refreshAllSites()))
      .then((fn) => {
        if (disposed) fn();
        else unlisten = fn;
      })
      .catch(() => undefined);
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [refreshAllSites]);

  useEffect(() => {
    if (!settingsLoaded || restoreResultReadRef.current) return;
    restoreResultReadRef.current = true;
    const language = useSettingsStore.getState().settings.language;
    void i18n.changeLanguage(language)
      .then(() => invoke<RestoreStartupResult | null>("take_restore_result"))
      .then((result) => {
        if (!result) return;
        if (result.status === "applied") {
          message.success(t("settings.webdav.restoreApplied"));
        } else {
          console.error("Application restore failed safely:", result.message);
          message.error(t("settings.webdav.restoreFailed"));
        }
      })
      .catch((error) => {
        console.error("Could not read application restore result:", error);
      });
  }, [i18n, message, settingsLoaded, t]);

  useEffect(() => {
    const root = document.documentElement;
    root.style.setProperty("--border-color", token.colorBorderSecondary);
    root.style.setProperty("--color-bg-container", token.colorBgContainer);
    root.style.setProperty("--color-bg-elevated", token.colorBgElevated);
    root.style.setProperty("--color-text", token.colorText);
    root.style.setProperty("--color-text-secondary", token.colorTextSecondary);
    root.style.setProperty("--color-primary", token.colorPrimary);
    root.style.setProperty("--scrollbar-thumb", token.colorTextQuaternary);
    root.style.setProperty("--scrollbar-thumb-hover", token.colorTextTertiary);
    document.body.style.backgroundColor = token.colorBgContainer;
  }, [token]);

  useEffect(() => {
    if (!isTauri() || !navigator.userAgent.includes("Windows")) return;
    void invoke("sync_windows_chrome", {
      dark: isDark,
      bg: token.colorBgContainer,
    });
  }, [isDark, token.colorBgContainer]);

  return (
    <div ref={rootRef} className="flex h-full flex-col">
      <TitleBar />
      <Layout className="min-h-0 flex-1 overflow-hidden" style={{ background: token.colorBgContainer, minHeight: 0 }}>
        <div className="flex min-h-0 flex-1 overflow-hidden">
          <SideNav />
          <main className="min-h-0 min-w-0 flex-1 overflow-hidden">
            <KeepAlivePages activePage={activePage} />
          </main>
        </div>
      </Layout>
    </div>
  );
}

export default function App() {
  const themeMode = useSettingsStore((s) => s.settings.themeMode);
  const primaryColor = useSettingsStore((s) => s.settings.primaryColor);
  const language = useSettingsStore((s) => s.settings.language);
  const isDark = useResolvedDarkMode(themeMode);

  return (
    <ConfigProvider
      locale={language === "en-US" ? enUS : zhCN}
      theme={{
        algorithm: isDark ? theme.darkAlgorithm : theme.defaultAlgorithm,
        token: {
          colorPrimary: primaryColor || "#1677ff",
          borderRadius: 8,
        },
      }}
      modal={{
        centered: true,
        styles: {
          // 遮罩不要 backdrop-filter：它会对整个窗口做一次模糊合成，在 175%
          // 缩放 + 大列表页上明显掉帧。半透明底色（rgba）已经足够把弹窗压下去。
          container: {
            maxHeight: "calc(100vh - 32px)",
            display: "flex",
            flexDirection: "column",
            overflow: "hidden",
          },
          body: {
            overflowY: "auto",
            overflowX: "hidden",
            minHeight: 0,
          },
        },
      }}
    >
      <AntdApp className="h-full">
        <AppInner isDark={isDark} />
      </AntdApp>
    </ConfigProvider>
  );
}
