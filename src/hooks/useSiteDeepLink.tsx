import { useCallback, useEffect, useRef } from "react";
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { invoke, isAppError, isTauri } from "@/lib/invoke";
import {
  getSiteDeepLinkKeyPrefix,
  hasSiteDeepLinkScheme,
  parseSiteDeepLink,
  type SiteDeepLinkPayload,
} from "@/lib/siteDeepLink";
import {
  consumeStartupDeepLinkUrls,
  rememberHandledDeepLink,
} from "@/lib/deepLinkSession";
import {
  anyCodexCapabilityOn,
  summarizeCodexCapabilities,
  codexFlagsFromCapabilities,
} from "@/lib/siteCapabilities";
import type { SiteCapabilities } from "@/types/domain";
import { useSiteStore, useUIStore } from "@/stores";
import type { DeepLinkSiteImportInput, DeepLinkSiteImportResult, Site } from "@/types/domain";

interface ModalLike {
  confirm: (config: {
    title: string;
    content: ReactNode;
    okText: string;
    cancelText: string;
    onOk: () => Promise<void>;
  }) => unknown;
}

interface MessageLike {
  success: (content: string) => unknown;
  error: (content: string) => unknown;
  info: (content: string) => unknown;
}

export interface ConfirmSiteDeepLinkDeps {
  modal: ModalLike;
  message: MessageLike;
  setPage: (page: "sites") => void;
  setSelectedSiteId: (id: string | null) => void;
  setPendingSiteForm: (payload: SiteDeepLinkPayload | null) => void;
  importSite: (input: DeepLinkSiteImportInput) => Promise<DeepLinkSiteImportResult>;
  onCreated?: (site: Site) => Promise<void> | void;
  t: (key: string) => string;
}

function getSuccessMessageKey(result: DeepLinkSiteImportResult): string {
  if (result.created) return "sites.deepLinkCreated";
  if (result.addedApiKey) return "sites.deepLinkAddedKey";
  if (result.activatedApiKey) return "sites.deepLinkActivatedKey";
  return "sites.deepLinkReused";
}

function capabilityTitleKey(key: string): string {
  if (key === "codex-compact") return "apply.remoteCompaction";
  if (key === "codex-vision") return "apply.imageUnderstanding";
  if (key === "codex-imagegen") return "apply.imageGeneration";
  if (key === "codex-search") return "apply.webSearch";
  return key;
}

function summarizeCapabilityPayload(
  capabilities: SiteCapabilities,
  t: ConfirmSiteDeepLinkDeps["t"],
): string {
  if (!anyCodexCapabilityOn(capabilities)) return t("sites.deepLinkCapabilitiesOff");
  return summarizeCodexCapabilities(codexFlagsFromCapabilities(capabilities))
    .filter((item) => item.on)
    .map((item) => t(capabilityTitleKey(item.key)))
    .join(" · ");
}

function protocolLabelKey(protocol: SiteDeepLinkPayload["protocol"]): string {
  return protocol === "anthropic" ? "sites.protocolAnthropic" : "sites.protocolOpenai";
}

function ConfirmRow({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex gap-2 text-sm">
      <span className="w-28 shrink-0 opacity-50">{label}</span>
      <div className="min-w-0 flex-1 break-all">{children}</div>
    </div>
  );
}

export function SiteDeepLinkConfirmContent({
  payload,
  t,
}: {
  payload: SiteDeepLinkPayload;
  t: ConfirmSiteDeepLinkDeps["t"];
}) {
  return (
    <div className="space-y-2">
      <ConfirmRow label={t("sites.name")}>{payload.name}</ConfirmRow>
      <ConfirmRow label={t("sites.baseUrls")}>
        <div className="space-y-1">
          {payload.baseUrls.map((url, i) => (
            <div key={`${i}-${url}`}>{url}</div>
          ))}
          <div className="text-xs opacity-50">{t("sites.baseUrlDefaultHint")}</div>
        </div>
      </ConfirmRow>
      <ConfirmRow label={t("sites.protocol")}>{t(protocolLabelKey(payload.protocol))}</ConfirmRow>
      {payload.notes ? <ConfirmRow label={t("sites.notes")}>{payload.notes}</ConfirmRow> : null}
      {payload.hasCapabilityParams ? (
        <ConfirmRow label={t("sites.codexPrivateCapabilities")}>
          {summarizeCapabilityPayload(payload.capabilities, t)}
        </ConfirmRow>
      ) : null}
      <ConfirmRow label={t("sites.apiKey")}>
        {payload.apiKey ? getSiteDeepLinkKeyPrefix(payload.apiKey) : t("sites.deepLinkNoKey")}
      </ConfirmRow>
      <div className="pt-1 text-xs opacity-50">{t("sites.deepLinkSecurityHint")}</div>
    </div>
  );
}

export function confirmSiteDeepLinkImport(
  payload: SiteDeepLinkPayload,
  deps: ConfirmSiteDeepLinkDeps,
) {
  deps.setPage("sites");

  deps.modal.confirm({
    title: deps.t("sites.deepLinkConfirmTitle"),
    content: <SiteDeepLinkConfirmContent payload={payload} t={deps.t} />,
    okText: deps.t("common.confirm"),
    cancelText: deps.t("common.cancel"),
    onOk: async () => {
      if (!payload.apiKey) {
        deps.setPendingSiteForm(payload);
        deps.message.info(deps.t("sites.deepLinkNeedKey"));
        return;
      }
      try {
        const result = await deps.importSite({
          name: payload.name,
          baseUrls: payload.baseUrls,
          apiKey: payload.apiKey,
          protocol: payload.protocol,
          notes: payload.notes,
          capabilities: payload.hasCapabilityParams ? payload.capabilities : undefined,
          keyName: payload.keyName,
        });
        deps.setSelectedSiteId(result.site.id);
        if (result.created) {
          await deps.onCreated?.(result.site);
        }
        deps.message.success(deps.t(getSuccessMessageKey(result)));
      } catch (e) {
        const detail = isAppError(e) ? e.message : String(e);
        deps.message.error(`${deps.t("sites.deepLinkImportFailed")}: ${detail}`);
        throw e;
      }
    },
  });
}

export function useSiteDeepLink({ modal, message }: { modal: ModalLike; message: MessageLike }) {
  const { t } = useTranslation();
  const translate = useCallback((key: string) => t(key), [t]);
  const importSite = useSiteStore((s) => s.importSiteFromDeepLink);
  const fetchModels = useSiteStore((s) => s.fetchModels);
  const setSelectedModel = useSiteStore((s) => s.setSelectedModel);
  const setPage = useUIStore((s) => s.setPage);
  const setSelectedSiteId = useUIStore((s) => s.setSelectedSiteId);
  const setPendingSiteForm = useUIStore((s) => s.setPendingSiteForm);
  const lastUrl = useRef<{ raw: string; at: number } | null>(null);

  const handleCreated = useCallback(
    async (site: Site) => {
      try {
        const result = await fetchModels(site.id);
        if (!site.selectedModelId && result.models[0]) {
          await setSelectedModel(site.id, result.models[0].modelId);
        }
      } catch {
        // Import already succeeded; model fetch is best-effort like manual create.
      }
    },
    [fetchModels, setSelectedModel],
  );

  useEffect(() => {
    if (!isTauri()) return;

    let disposed = false;
    let unlisten: (() => void) | null = null;

    const shouldHandle = (raw: string) => {
      const now = Date.now();
      if (lastUrl.current && lastUrl.current.raw === raw && now - lastUrl.current.at < 2000) {
        return false;
      }
      lastUrl.current = { raw, at: now };
      return true;
    };

    const handleUrls = (urls: string[] | null | undefined) => {
      if (disposed) return;
      for (const raw of urls ?? []) {
        if (!shouldHandle(raw)) continue;
        rememberHandledDeepLink(raw);
        const payload = parseSiteDeepLink(raw);
        if (!payload) {
          if (hasSiteDeepLinkScheme(raw)) {
            message.error(translate("sites.deepLinkInvalid"));
          }
          continue;
        }
        confirmSiteDeepLinkImport(payload, {
          modal,
          message,
          setPage,
          setSelectedSiteId,
          setPendingSiteForm,
          importSite,
          onCreated: handleCreated,
          t: translate,
        });
        break;
      }
    };

    const setup = async () => {
      try {
        const { getCurrent, onOpenUrl } = await import("@tauri-apps/plugin-deep-link");
        const pending = await invoke<string | null>("take_pending_deep_link");
        const startup = consumeStartupDeepLinkUrls((await getCurrent()) ?? []);
        handleUrls([...(pending ? [pending] : []), ...startup]);
        const stopPlugin = await onOpenUrl((urls) => {
          void invoke("restore_main_window").catch(() => undefined);
          handleUrls(urls);
        });
        // 写 pending-deeplink.url 的只有 macOS 分支，别的平台轮询一个永远不会出现的
        // 文件纯属浪费（每 400ms 一次 IPC）。读不到能力时按「需要」处理，保住 macOS 行为。
        const needsPolling = await invoke<boolean>("deep_link_requires_polling").catch(
          () => true,
        );
        const poll = needsPolling
          ? window.setInterval(() => {
              if (disposed) return;
              void invoke<string | null>("take_pending_deep_link")
                .then((url) => {
                  if (url) handleUrls([url]);
                })
                .catch(() => undefined);
            }, 400)
          : null;
        unlisten = () => {
          if (poll !== null) window.clearInterval(poll);
          stopPlugin();
        };
      } catch (e) {
        console.warn("Failed to initialize deep link listener:", e);
      }
    };

    void setup();

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [
    handleCreated,
    importSite,
    message,
    modal,
    setPage,
    setPendingSiteForm,
    setSelectedSiteId,
    translate,
  ]);
}
