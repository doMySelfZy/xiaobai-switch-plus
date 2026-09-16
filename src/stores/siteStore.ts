import { create } from "zustand";
import { invoke } from "@/lib/invoke";
import type {
  AddSiteApiKeyInput,
  CreateSiteInput,
  DeepLinkSiteImportInput,
  DeepLinkSiteImportResult,
  FetchModelsResult,
  Site,
  SiteModel,
  SiteQuota,
  SwitchRouteResult,
  SwitchSiteApiKeyResult,
  UpdateSiteApiKeyInput,
  UpdateSiteInput,
} from "@/types/domain";
import { activeApiKeyId } from "@/lib/siteApiKey";
import { originFromBaseUrl, invalidateSiteIconCache } from "@/lib/siteIcon";
import { isQuotaCacheFresh, quotaCacheKey } from "@/lib/quotaProbe";
import { useApplyStore } from "./applyStore";

const quotaInflight = new Map<string, Promise<SiteQuota>>();

export function resetQuotaInflight() {
  quotaInflight.clear();
}

interface SiteState {
  sites: Site[];
  modelsBySite: Record<string, SiteModel[]>;
  /** Per-site model list fetch in progress. */
  modelsLoadingBySite: Record<string, boolean>;
  quotaBySite: Record<string, SiteQuota>;
  quotaAttemptBySite: Record<string, SiteQuota>;
  quotaCacheKeyBySite: Record<string, string>;
  quotaAttemptCacheKeyBySite: Record<string, string>;
  quotaLoadingBySite: Record<string, boolean>;
  loading: boolean;
  /** True after at least one successful sites load. */
  hydrated: boolean;
  /** True while any site is fetching models (aggregate; prefer the per-site maps). */
  fetchingModels: boolean;
  fetchingModelsByKey: Record<string, boolean>;
  /**
   * Per-site "models are being refreshed" flag (covers both `fetchModels` and
   * `switchApiKey`). The previous list is intentionally kept in `modelsBySite`
   * while this is true — see the comment in `switchApiKey`.
   */
  fetchingModelsBySite: Record<string, boolean>;
  error: string | null;
  loadSites: (opts?: { force?: boolean; soft?: boolean }) => Promise<void>;
  getSiteApiKey: (id: string, apiKeyId?: string) => Promise<string>;
  createSite: (input: CreateSiteInput) => Promise<Site>;
  importSiteFromDeepLink: (input: DeepLinkSiteImportInput) => Promise<DeepLinkSiteImportResult>;
  updateSite: (id: string, input: UpdateSiteInput) => Promise<Site>;
  addApiKey: (siteId: string, input: AddSiteApiKeyInput) => Promise<Site>;
  updateApiKey: (siteId: string, apiKeyId: string, input: UpdateSiteApiKeyInput) => Promise<Site>;
  deleteApiKey: (siteId: string, apiKeyId: string) => Promise<Site>;
  switchApiKey: (
    siteId: string,
    apiKeyId: string,
    opts?: { syncTargets?: boolean },
  ) => Promise<SwitchSiteApiKeyResult>;
  switchRoute: (
    siteId: string,
    baseUrl: string,
    opts?: { apply?: boolean },
  ) => Promise<SwitchRouteResult>;
  deleteSite: (id: string, cleanupTargets?: boolean) => Promise<void>;
  reorderSites: (ids: string[]) => Promise<void>;
  fetchModels: (siteId: string) => Promise<FetchModelsResult>;
  listModels: (siteId: string, opts?: { force?: boolean }) => Promise<SiteModel[]>;
  probeQuota: (siteId: string, opts?: { force?: boolean }) => Promise<SiteQuota>;
  setSelectedModel: (siteId: string, modelId: string) => Promise<void>;
  deleteModel: (siteId: string, modelId: string) => Promise<void>;
  clearModels: (siteId: string) => Promise<void>;
}

function withoutSite<T>(record: Record<string, T>, siteId: string): Record<string, T> {
  const next = { ...record };
  delete next[siteId];
  return next;
}

/**
 * `${siteId}:${apiKeyId}` 形式的在途标记 → per-site 视图。
 * 站点 id 是 UUID（不含冒号），所以前缀切分是可靠的。
 */
function sitesWithInFlightFetch(loading: Record<string, boolean>): Record<string, boolean> {
  const bySite: Record<string, boolean> = {};
  for (const [key, busy] of Object.entries(loading)) {
    if (!busy) continue;
    const sep = key.indexOf(":");
    if (sep > 0) bySite[key.slice(0, sep)] = true;
  }
  return bySite;
}

function clearSiteQuotaState(state: SiteState, siteId: string) {
  for (const key of quotaInflight.keys()) {
    if (key.startsWith(`${siteId}:`)) quotaInflight.delete(key);
  }
  return {
    quotaBySite: withoutSite(state.quotaBySite, siteId),
    quotaAttemptBySite: withoutSite(state.quotaAttemptBySite, siteId),
    quotaCacheKeyBySite: withoutSite(state.quotaCacheKeyBySite, siteId),
    quotaAttemptCacheKeyBySite: withoutSite(state.quotaAttemptCacheKeyBySite, siteId),
    quotaLoadingBySite: withoutSite(state.quotaLoadingBySite, siteId),
  };
}

export const useSiteStore = create<SiteState>((set, get) => ({
  sites: [],
  modelsBySite: {},
  modelsLoadingBySite: {},
  quotaBySite: {},
  quotaAttemptBySite: {},
  quotaCacheKeyBySite: {},
  quotaAttemptCacheKeyBySite: {},
  quotaLoadingBySite: {},
  loading: false,
  hydrated: false,
  fetchingModels: false,
  fetchingModelsByKey: {},
  fetchingModelsBySite: {},
  error: null,
  loadSites: async (opts) => {
    const hasCache = get().hydrated;
    // soft / cached: refresh without flipping the page into a loading skeleton
    if ((opts?.soft || hasCache) && !opts?.force) {
      try {
        const sites = await invoke<Site[]>("list_sites");
        set({ sites, hydrated: true, error: null });
      } catch (e) {
        // Soft refresh failures should not clear a hydrated list.
        if (!hasCache) {
          set({ error: String(e) });
          throw e;
        }
      }
      return;
    }
    set({ loading: true, error: null });
    try {
      const sites = await invoke<Site[]>("list_sites");
      set({ sites, hydrated: true });
    } catch (e) {
      set({ error: String(e) });
      throw e;
    } finally {
      set({ loading: false });
    }
  },
  getSiteApiKey: (id, apiKeyId) => invoke<string>("get_site_api_key", { id, apiKeyId }),
  createSite: async (input) => {
    const site = await invoke<Site>("create_site", { input });
    set({ sites: [...get().sites, site], hydrated: true });
    return site;
  },
  importSiteFromDeepLink: async (input) => {
    const result = await invoke<DeepLinkSiteImportResult>("import_site_from_deep_link", {
      input,
    });
    const sites = get().sites;
    const idx = sites.findIndex((s) => s.id === result.site.id);
    const previous = idx >= 0 ? sites[idx] : undefined;
    const quotaConfigChanged = Boolean(
      previous && quotaCacheKey(previous) !== quotaCacheKey(result.site),
    );
    set({
      sites:
        idx >= 0
          ? sites.map((s) => (s.id === result.site.id ? result.site : s))
          : [...sites, result.site],
      hydrated: true,
      ...(quotaConfigChanged ? clearSiteQuotaState(get(), result.site.id) : {}),
    });
    return result;
  },
  updateSite: async (id, input) => {
    const previous = get().sites.find((s) => s.id === id);
    const site = await invoke<Site>("update_site", { id, input });
    const quotaConfigChanged = Boolean(
      previous && quotaCacheKey(previous) !== quotaCacheKey(site),
    );
    set({
      sites: get().sites.map((s) => (s.id === id ? site : s)),
      ...(quotaConfigChanged ? clearSiteQuotaState(get(), id) : {}),
    });
    return site;
  },
  addApiKey: async (siteId, input) => {
    const site = await invoke<Site>("add_site_api_key", { siteId, input });
    set({ sites: get().sites.map((s) => (s.id === siteId ? site : s)) });
    return site;
  },
  updateApiKey: async (siteId, apiKeyId, input) => {
    const previous = get().sites.find((s) => s.id === siteId);
    const site = await invoke<Site>("update_site_api_key", { siteId, apiKeyId, input });
    set({
      sites: get().sites.map((s) => (s.id === siteId ? site : s)),
      ...(previous && quotaCacheKey(previous) !== quotaCacheKey(site)
        ? clearSiteQuotaState(get(), siteId)
        : {}),
    });
    return site;
  },
  deleteApiKey: async (siteId, apiKeyId) => {
    const site = await invoke<Site>("delete_site_api_key", { siteId, apiKeyId });
    set({ sites: get().sites.map((s) => (s.id === siteId ? site : s)) });
    return site;
  },
  switchApiKey: async (siteId, apiKeyId, opts) => {
    const prev = get().sites.find((s) => s.id === siteId);
    const fetchKey = `${siteId}:${apiKeyId}`;
    const optimistic = prev
      ? {
          ...prev,
          activeApiKeyId: apiKeyId,
          apiKeys: (prev.apiKeys ?? []).map((key) => ({ ...key, isActive: key.id === apiKeyId })),
        }
      : null;
    const inFlightKeys = { ...get().fetchingModelsByKey, [fetchKey]: true };
    set({
      fetchingModels: true,
      fetchingModelsByKey: inFlightKeys,
      fetchingModelsBySite: sitesWithInFlightFetch(inFlightKeys),
      // 刻意不清空 modelsBySite[siteId]：新密钥的模型要等后端拉完才回来，清空会让
      // 详情面板退回骨架屏（SitesPage 用「有没有缓存条目」判断），用户会盯着空白等
      // 几十秒。刷新状态由 fetchingModelsBySite 表达，旧列表继续显示到新列表落地。
      ...(optimistic
        ? { sites: get().sites.map((s) => (s.id === siteId ? optimistic : s)) }
        : {}),
    });
    try {
      const result = await invoke<SwitchSiteApiKeyResult>("switch_site_api_key", {
        siteId,
        apiKeyId,
        syncTargets: opts?.syncTargets === true,
      });
      const current = result.site;
      const stillCurrent = activeApiKeyId(current) === apiKeyId;
      set({
        sites: get().sites.map((s) => (s.id === siteId ? current : s)),
        ...(stillCurrent
          ? { modelsBySite: { ...get().modelsBySite, [siteId]: result.models } }
          : {}),
        ...(prev && quotaCacheKey(prev) !== quotaCacheKey(current)
          ? clearSiteQuotaState(get(), siteId)
          : {}),
      });
      void useApplyStore.getState().loadStatus({ background: true }).catch(() => null);
      return result;
    } finally {
      const loading = { ...get().fetchingModelsByKey };
      delete loading[fetchKey];
      set({
        fetchingModelsByKey: loading,
        fetchingModelsBySite: sitesWithInFlightFetch(loading),
        fetchingModels: Object.values(loading).some(Boolean),
      });
    }
  },
  switchRoute: async (siteId, baseUrl, opts) => {
    const prev = get().sites.find((s) => s.id === siteId);
    const result = await invoke<SwitchRouteResult>("switch_site_route", {
      siteId,
      baseUrl,
      apply: opts?.apply !== false,
    });
    set({
      sites: get().sites.map((s) => (s.id === siteId ? result.site : s)),
      ...(prev && quotaCacheKey(prev) !== quotaCacheKey(result.site)
        ? clearSiteQuotaState(get(), siteId)
        : {}),
    });
    const prevOrigin = prev ? originFromBaseUrl(prev.baseUrl) : null;
    const nextOrigin = originFromBaseUrl(result.site.baseUrl);
    if (prevOrigin !== nextOrigin) {
      invalidateSiteIconCache(siteId);
    }
    void useApplyStore.getState().loadStatus({ background: true }).catch(() => null);
    return result;
  },
  deleteSite: async (id, cleanupTargets = false) => {
    await invoke("delete_site", { id, cleanupTargets });
    const modelsBySite = { ...get().modelsBySite };
    delete modelsBySite[id];
    const modelsLoadingBySite = { ...get().modelsLoadingBySite };
    delete modelsLoadingBySite[id];
    set({
      sites: get().sites.filter((s) => s.id !== id),
      modelsBySite,
      modelsLoadingBySite,
      ...clearSiteQuotaState(get(), id),
    });
  },
  reorderSites: async (ids) => {
    const previous = get().sites;
    const byId = new Map(previous.map((s) => [s.id, s]));
    const next: Site[] = [];
    const seen = new Set<string>();
    ids.forEach((id, index) => {
      const site = byId.get(id);
      if (!site) return;
      seen.add(id);
      next.push({ ...site, sortOrder: index });
    });
    for (const site of previous) {
      if (!seen.has(site.id)) next.push(site);
    }
    set({ sites: next });
    try {
      await invoke("reorder_sites", { ids });
      await get().loadSites({ soft: true });
    } catch (e) {
      set({ sites: previous });
      throw e;
    }
  },
  fetchModels: async (siteId) => {
    const site = get().sites.find((s) => s.id === siteId);
    const apiKeyId = activeApiKeyId(site ?? null);
    const fetchKey = `${siteId}:${apiKeyId ?? "active"}`;
    const inFlightKeys = { ...get().fetchingModelsByKey, [fetchKey]: true };
    set({
      fetchingModels: true,
      fetchingModelsByKey: inFlightKeys,
      // 同 switchApiKey：刷新期间保留旧列表，per-site 标记表达「正在刷新」。
      fetchingModelsBySite: sitesWithInFlightFetch(inFlightKeys),
      error: null,
    });
    try {
      const result = await invoke<FetchModelsResult>("fetch_site_models", { siteId, apiKeyId });
      let models = Array.isArray(result.models) ? result.models : [];
      if (models.length === 0) {
        const listed = await invoke<SiteModel[]>("list_site_models", { siteId, apiKeyId });
        if (Array.isArray(listed) && listed.length > 0) models = listed;
      }
      const current = get().sites.find((s) => s.id === siteId);
      const resultKey = result.apiKeyId || apiKeyId;
      const stillCurrent = !resultKey || activeApiKeyId(current ?? null) === resultKey;
      if (stillCurrent) {
        set({
          modelsBySite: { ...get().modelsBySite, [siteId]: models },
          sites: get().sites.map((s) =>
            s.id === siteId
              ? {
                  ...s,
                  lastModelFetchAt: result.fetchedAt,
                  lastModelFetchLatencyMs: result.latencyMs,
                  lastModelFetchError: null,
                }
              : s,
          ),
        });
      }
      return { ...result, models };
    } catch (e) {
      const msg =
        typeof e === "object" && e && "message" in e
          ? String((e as { message: string }).message)
          : String(e);
      const current = get().sites.find((s) => s.id === siteId);
      if (!apiKeyId || activeApiKeyId(current ?? null) === apiKeyId) {
        set({
          error: msg,
          sites: get().sites.map((s) =>
            s.id === siteId ? { ...s, lastModelFetchError: msg } : s,
          ),
        });
      }
      throw e;
    } finally {
      const loading = { ...get().fetchingModelsByKey };
      delete loading[fetchKey];
      set({
        fetchingModelsByKey: loading,
        fetchingModelsBySite: sitesWithInFlightFetch(loading),
        fetchingModels: Object.values(loading).some(Boolean),
      });
    }
  },
  listModels: async (siteId, opts) => {
    const site = get().sites.find((s) => s.id === siteId);
    const apiKeyId = activeApiKeyId(site ?? null);
    const cached = get().modelsBySite[siteId];
    const cachedKey = cached?.[0]?.apiKeyId;
    if (
      !opts?.force &&
      Object.prototype.hasOwnProperty.call(get().modelsBySite, siteId) &&
      (!apiKeyId || !cachedKey || cachedKey === apiKeyId)
    ) {
      return cached ?? [];
    }
    set({
      modelsLoadingBySite: { ...get().modelsLoadingBySite, [siteId]: true },
    });
    try {
      const models = await invoke<SiteModel[]>("list_site_models", { siteId, apiKeyId });
      const list = Array.isArray(models) ? models : [];
      const current = get().modelsBySite[siteId];
      // Don't let a stale list overwrite a newer non-empty fetch.
      if (!opts?.force && Array.isArray(current) && current.length > 0 && list.length === 0) {
        return current;
      }
      set({ modelsBySite: { ...get().modelsBySite, [siteId]: list } });
      return list;
    } catch (e) {
      // Cache empty list so UI can leave skeleton state even on failure.
      if (!Object.prototype.hasOwnProperty.call(get().modelsBySite, siteId)) {
        set({ modelsBySite: { ...get().modelsBySite, [siteId]: [] } });
      }
      throw e;
    } finally {
      set({
        modelsLoadingBySite: { ...get().modelsLoadingBySite, [siteId]: false },
      });
    }
  },
  probeQuota: async (siteId, opts) => {
    const site = get().sites.find((s) => s.id === siteId);
    if (!site) {
      throw { code: "not_found", message: "site not found" };
    }
    const key = quotaCacheKey(site);
    const pending = quotaInflight.get(key);
    if (pending) {
      return pending;
    }
    const cached = get().quotaAttemptBySite[siteId];
    const cachedKey = get().quotaAttemptCacheKeyBySite[siteId];
    if (
      !opts?.force &&
      cached &&
      cachedKey === key &&
      isQuotaCacheFresh(cached)
    ) {
      return cached;
    }

    set({
      quotaLoadingBySite: { ...get().quotaLoadingBySite, [siteId]: true },
    });
    let run: Promise<SiteQuota>;
    const storeIfCurrent = (quota: SiteQuota) => {
      const current = get().sites.find((s) => s.id === siteId);
      if (quotaInflight.get(key) === run && current && quotaCacheKey(current) === key) {
        const next = {
          quotaAttemptBySite: { ...get().quotaAttemptBySite, [siteId]: quota },
          quotaAttemptCacheKeyBySite: {
            ...get().quotaAttemptCacheKeyBySite,
            [siteId]: key,
          },
        };
        set(
          quota.status === "available"
            ? {
                ...next,
                quotaBySite: { ...get().quotaBySite, [siteId]: quota },
                quotaCacheKeyBySite: { ...get().quotaCacheKeyBySite, [siteId]: key },
              }
            : next,
        );
      }
      return quota;
    };
    run = invoke<SiteQuota>("probe_site_quota", { siteId })
      .then(storeIfCurrent)
      .catch((e) => {
        const message =
          typeof e === "object" && e && "message" in e
            ? String((e as { message: string }).message)
            : String(e);
        return storeIfCurrent({
          status: "error",
          remainingUsd: null,
          usedUsd: null,
          totalUsd: null,
          unlimited: false,
          unit: null,
          expiresAt: null,
          source: null,
          endpoint: null,
          fetchedAt: Date.now(),
          latencyMs: 0,
          error: message,
        });
      })
      .finally(() => {
        if (quotaInflight.get(key) === run) quotaInflight.delete(key);
        const current = get().sites.find((s) => s.id === siteId);
        const loading = { ...get().quotaLoadingBySite };
        if (current) {
          loading[siteId] = quotaInflight.has(quotaCacheKey(current));
        } else {
          delete loading[siteId];
        }
        set({
          quotaLoadingBySite: loading,
        });
      });
    quotaInflight.set(key, run);
    return run;
  },
  setSelectedModel: async (siteId, modelId) => {
    await invoke("set_selected_model", { siteId, modelId });
    set((state) => {
      const cached = Object.prototype.hasOwnProperty.call(state.modelsBySite, siteId);
      const list = state.modelsBySite[siteId] ?? [];
      const has = list.some((m) => m.modelId === modelId);
      return {
        sites: state.sites.map((s) =>
          s.id === siteId ? { ...s, selectedModelId: modelId } : s,
        ),
        modelsBySite:
          cached && !has
            ? {
                ...state.modelsBySite,
                [siteId]: [
                  ...list,
                  {
                    id: modelId,
                    siteId,
                    modelId,
                    displayName: modelId,
                    ownedBy: null,
                    raw: null,
                    isManual: true,
                  },
                ],
              }
            : state.modelsBySite,
      };
    });
  },
  deleteModel: async (siteId, modelId) => {
    const site = await invoke<Site>("delete_site_model", { siteId, modelId });
    set({
      sites: get().sites.map((s) => (s.id === siteId ? site : s)),
      modelsBySite: {
        ...get().modelsBySite,
        [siteId]: (get().modelsBySite[siteId] ?? []).filter((m) => m.modelId !== modelId),
      },
    });
  },
  clearModels: async (siteId) => {
    const site = await invoke<Site>("clear_site_models", { siteId });
    set({
      sites: get().sites.map((s) => (s.id === siteId ? site : s)),
      modelsBySite: { ...get().modelsBySite, [siteId]: [] },
    });
  },
}));
