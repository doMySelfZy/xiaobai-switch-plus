import { create } from "zustand";
import { invoke, isTauri } from "@/lib/invoke";
import type {
  AddSiteApiKeyInput,
  CreateSiteInput,
  DeepLinkSiteImportInput,
  DeepLinkSiteImportResult,
  FetchModelsResult,
  RefreshAllSitesResult,
  RefreshSiteResult,
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
const modelInflight = new Map<string, Promise<FetchModelsResult>>();
const modelRequestVersion = new Map<string, number>();
const MAX_MODEL_REFRESH_CONCURRENCY = 4;
/** 行内刷新指示器最短露出时间：两阶段都命中缓存时不让它闪一下就没。 */
const MIN_REFRESH_INDICATOR_MS = 300;
let refreshAllInflight: Promise<RefreshAllSitesResult> | null = null;

function modelFetchKey(
  siteId: string,
  apiKeyId: string | null,
  baseUrl: string,
  quotaRevision: string,
): string {
  return `${siteId}:${apiKeyId ?? "active"}:${baseUrl}:${quotaRevision}`;
}

function nextModelRequestVersion(siteId: string): number {
  const next = (modelRequestVersion.get(siteId) ?? 0) + 1;
  modelRequestVersion.set(siteId, next);
  return next;
}

export function resetQuotaInflight() {
  quotaInflight.clear();
  modelInflight.clear();
  modelRequestVersion.clear();
  refreshAllInflight = null;
}

async function emitSitesRefreshFinished() {
  if (!isTauri()) return;
  try {
    const { emit } = await import("@tauri-apps/api/event");
    await emit("sites-refresh-finished");
  } catch {
    // Browser mode and older runtimes may not expose the event bridge.
  }
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
  /** True while the shared model + quota refresh is running. */
  refreshingAll: boolean;
  fetchingModelsByKey: Record<string, boolean>;
  /** Site IDs currently being refreshed (models + quota). */
  refreshingSiteIds: string[];
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
  fetchModels: (siteId: string, apiKeyId?: string | null) => Promise<FetchModelsResult>;
  refreshAllSites: () => Promise<RefreshAllSitesResult>;
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

function errorMessage(error: unknown): string {
  if (typeof error === "object" && error && "message" in error) {
    return String((error as { message: unknown }).message);
  }
  return String(error);
}

/**
 * 余额命令 reject 时合成的尝试记录。
 *
 * 列表行第二行要能说出「为什么没有余额」，只写成功结果会让连不上的站点永远空白。
 */
function errorQuotaAttempt(message: string): SiteQuota {
  return {
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
  };
}

async function mapWithConcurrency<T, R>(
  items: T[],
  limit: number,
  worker: (item: T) => Promise<R>,
): Promise<R[]> {
  const results = new Array<R>(items.length);
  let cursor = 0;
  const runWorker = async () => {
    while (true) {
      const index = cursor++;
      if (index >= items.length) return;
      results[index] = await worker(items[index]);
    }
  };
  await Promise.all(
    Array.from({ length: Math.min(limit, items.length) }, () => runWorker()),
  );
  return results;
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
  refreshingAll: false,
  fetchingModelsByKey: {},
  fetchingModelsBySite: {},
  refreshingSiteIds: [],
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
  fetchModels: (siteId, requestedApiKeyId) => {
    const site = get().sites.find((s) => s.id === siteId);
    const apiKeyId = requestedApiKeyId === undefined
      ? activeApiKeyId(site ?? null)
      : requestedApiKeyId;
    const baseUrl = site?.baseUrl ?? "";
    const quotaRevision = site?.quotaRevision ?? "";
    const fetchKey = modelFetchKey(siteId, apiKeyId, baseUrl, quotaRevision);
    const existing = modelInflight.get(fetchKey);
    if (existing) return existing;

    const requestVersion = nextModelRequestVersion(siteId);
    const inFlightKeys = { ...get().fetchingModelsByKey, [fetchKey]: true };
    set({
      fetchingModels: true,
      fetchingModelsByKey: inFlightKeys,
      // 同 switchApiKey：刷新期间保留旧列表，per-site 标记表达「正在刷新」。
      fetchingModelsBySite: sitesWithInFlightFetch(inFlightKeys),
      error: null,
    });

    let run!: Promise<FetchModelsResult>;
    run = (async (): Promise<FetchModelsResult> => {
      try {
        const result = await invoke<FetchModelsResult>("fetch_site_models", { siteId, apiKeyId });
        let models = Array.isArray(result.models) ? result.models : [];
        if (models.length === 0) {
          const listed = await invoke<SiteModel[]>("list_site_models", { siteId, apiKeyId });
          if (Array.isArray(listed) && listed.length > 0) models = listed;
        }
        const current = get().sites.find((s) => s.id === siteId);
        const resultKey = result.apiKeyId || apiKeyId;
        const stillCurrent =
          requestVersion === modelRequestVersion.get(siteId) &&
          current?.baseUrl === baseUrl &&
          (!resultKey || activeApiKeyId(current ?? null) === resultKey);
        if (stillCurrent) {
          const selectedModelId = current?.apiKeys?.find((key) => key.id === resultKey)?.selectedModelId
            ?? current?.selectedModelId
            ?? null;
          set({
            modelsBySite: { ...get().modelsBySite, [siteId]: models },
            sites: get().sites.map((s) =>
              s.id === siteId
                ? {
                    ...s,
                    selectedModelId,
                    lastModelFetchAt: result.fetchedAt,
                    lastModelFetchLatencyMs: result.latencyMs,
                    lastModelFetchError: null,
                    apiKeys: s.apiKeys?.map((key) =>
                      key.id === resultKey
                        ? { ...key, selectedModelId }
                        : key,
                    ),
                  }
                : s,
            ),
          });
        }
        return { ...result, models };
      } catch (e) {
        const msg = errorMessage(e);
        const current = get().sites.find((s) => s.id === siteId);
        const stillCurrent =
          requestVersion === modelRequestVersion.get(siteId) &&
          current?.baseUrl === baseUrl &&
          (!apiKeyId || activeApiKeyId(current ?? null) === apiKeyId);
        if (stillCurrent) {
          set({
            error: msg,
            // 清空模型列表，让站点显示为不可用（红点）
            modelsBySite: { ...get().modelsBySite, [siteId]: [] },
            sites: get().sites.map((s) =>
              s.id === siteId ? { ...s, lastModelFetchError: msg } : s,
            ),
          });
        }
        throw e;
      } finally {
        if (modelInflight.get(fetchKey) === run) modelInflight.delete(fetchKey);
        const loading = { ...get().fetchingModelsByKey };
        delete loading[fetchKey];
        set({
          fetchingModelsByKey: loading,
          fetchingModelsBySite: sitesWithInFlightFetch(loading),
          fetchingModels: Object.values(loading).some(Boolean),
        });
      }
    })();
    modelInflight.set(fetchKey, run);
    return run;
  },
  refreshAllSites: () => {
    if (refreshAllInflight) return refreshAllInflight;

    const run = (async (): Promise<RefreshAllSitesResult> => {
      if (!useSiteStore.getState().hydrated) {
        await get().loadSites({ soft: true });
      }
      const enabledSites = useSiteStore
        .getState()
        .sites.filter((site) => site.enabled)
        .map((site) => ({
          id: site.id,
          apiKeyId: activeApiKeyId(site),
          quotaKey: quotaCacheKey(site),
        }));

      set({ refreshingAll: true, refreshingSiteIds: enabledSites.map((site) => site.id) });
      try {
        // 一个站点的一轮刷新 = 模型 + 余额两件事，所以两阶段必须在同一个 worker 里等齐：
        // 余额曾经走批量命令、在模型循环之后才 await，于是所有行都停止转圈了头部按钮还在转。
        // `refresh_site_quota` 而不是 `probe_site_quota`：前者顺带写后端余额缓存，
        // 悬浮窗只读那份缓存（跨 webview 拿不到这里的 zustand 状态）。
        const sites = await mapWithConcurrency(
          enabledSites,
          MAX_MODEL_REFRESH_CONCURRENCY,
          async ({ id, apiKeyId, quotaKey }) => {
            const startTime = Date.now();
            const [model, quota] = await Promise.all([
              get()
                .fetchModels(id, apiKeyId)
                .then(
                  (result) => ({ modelCount: result.models.length, error: null }),
                  (error) => ({ modelCount: 0, error: errorMessage(error) }),
                ),
              invoke<SiteQuota>("refresh_site_quota", { siteId: id }).then(
                (value) => ({ value, error: null }),
                (error) => ({ value: null, error: errorMessage(error) }),
              ),
            ]);

            const current = get().sites.find((site) => site.id === id);
            // reject 也要留下尝试记录，否则连不上的站点在列表里永远是空白一行。
            const attempt =
              quota.value ?? errorQuotaAttempt(quota.error ?? "quota refresh failed");
            const quotaOk =
              quota.error === null && attempt.status === "available" && !attempt.error;
            // 站点配置在这轮里被改过（换密钥 / 换 Base URL）就不写回，避免旧响应覆盖新状态。
            if (current && quotaKey === quotaCacheKey(current)) {
              set({
                quotaAttemptBySite: { ...get().quotaAttemptBySite, [id]: attempt },
                quotaAttemptCacheKeyBySite: {
                  ...get().quotaAttemptCacheKeyBySite,
                  [id]: quotaKey,
                },
                ...(attempt.status === "available"
                  ? {
                      quotaBySite: { ...get().quotaBySite, [id]: attempt },
                      quotaCacheKeyBySite: { ...get().quotaCacheKeyBySite, [id]: quotaKey },
                    }
                  : {}),
              });
            }

            // 指示器至少露一面：两阶段都命中缓存时整轮可能连一帧都不到。
            const elapsed = Date.now() - startTime;
            if (elapsed < MIN_REFRESH_INDICATOR_MS) {
              await new Promise((resolve) =>
                setTimeout(resolve, MIN_REFRESH_INDICATOR_MS - elapsed),
              );
            }
            set((state) => ({
              refreshingSiteIds: state.refreshingSiteIds.filter((siteId) => siteId !== id),
            }));

            return {
              siteId: id,
              modelCount: model.modelCount,
              modelsOk: model.error === null,
              quotaOk,
              modelError: model.error,
              quotaError:
                quota.error ?? quota.value?.error ?? (quotaOk ? null : "quota refresh failed"),
            } satisfies RefreshSiteResult;
          },
        );
        const successCount = sites.filter((site) => site.modelsOk && site.quotaOk).length;
        const result = {
          sites,
          successCount,
          failureCount: sites.length - successCount,
        } satisfies RefreshAllSitesResult;
        await emitSitesRefreshFinished();
        return result;
      } finally {
        set({ refreshingAll: false });
      }
    })();

    refreshAllInflight = run;
    const clearInflight = () => {
      if (refreshAllInflight === run) refreshAllInflight = null;
    };
    void run.then(clearInflight, clearInflight);
    return run;
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
    const requestVersion = nextModelRequestVersion(siteId);
    set({
      modelsLoadingBySite: { ...get().modelsLoadingBySite, [siteId]: true },
    });
    try {
      const models = await invoke<SiteModel[]>("list_site_models", { siteId, apiKeyId });
      const list = Array.isArray(models) ? models : [];
      const current = get().modelsBySite[siteId];
      const currentSite = get().sites.find((entry) => entry.id === siteId);
      // A list read may race a network fetch or a site/key change. Only the
      // newest request for the same current key may replace the cache.
      if (
        requestVersion !== modelRequestVersion.get(siteId) ||
        activeApiKeyId(currentSite ?? null) !== apiKeyId
      ) {
        return current ?? list;
      }
      if (!opts?.force && Array.isArray(current) && current.length > 0 && list.length === 0) {
        return current;
      }
      set({ modelsBySite: { ...get().modelsBySite, [siteId]: list } });
      return list;
    } catch (e) {
      // Cache empty list so UI can leave skeleton state even on failure.
      if (
        requestVersion === modelRequestVersion.get(siteId) &&
        !Object.prototype.hasOwnProperty.call(get().modelsBySite, siteId)
      ) {
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
      .catch((e) => storeIfCurrent(errorQuotaAttempt(errorMessage(e))))
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
